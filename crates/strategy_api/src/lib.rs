pub mod orderbook;

use anyhow::Result;
use rengine_types::{
    Decimal, ExecutionRequest, MarketSpec, OrderInfo, PublicTrade, StrategyConfiguration,
    TopBookUpdate, VenueBookKey,
};
use std::{collections::HashMap, fmt::Arguments};

pub mod bindings {

    use wit_bindgen::generate;
    generate!({path: "strategy.wit", pub_export_macro: true,export_macro_name: "export", });
}

pub fn get_indicator(key: &str) -> Result<Decimal, String> {
    let value = bindings::indicator(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_balance(key: &str) -> Result<Decimal, String> {
    let value = bindings::balance(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_book(key: &str) -> Result<TopBookUpdate, String> {
    let value = bindings::book(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_open_orders(key: &str) -> Result<HashMap<String, OrderInfo>, String> {
    let value = bindings::open_orders(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_perp_position(key: &str) -> Result<Decimal, String> {
    let value = bindings::perp_positions(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_spot_exposure(key: &str) -> Result<Decimal, String> {
    let value = bindings::spot_exposure(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn get_trade_flow() -> Result<HashMap<VenueBookKey, Vec<PublicTrade>>, String> {
    let value = bindings::trade_flow()?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

/// Get the market specification for a trading instrument.
/// The key format is "venue|instrument" (e.g., "hyperliquid|eth/usdc-perp").
/// Returns contract size, price precision, minimum price, etc.
pub fn get_market_spec(key: &str) -> Result<MarketSpec, String> {
    let value = bindings::market_spec(key)?;

    borsh::from_slice(&value).map_err(|err| err.to_string())
}

pub fn trace(args: Arguments) {
    bindings::trace(&args.to_string());
}

// Now define a macro to use like printf!
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::trace(format_args!($($arg)*))
    };
}

pub trait Plugin {
    type State: borsh::BorshSerialize + borsh::BorshDeserialize + Default;

    fn init() -> StrategyConfiguration;
    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String>;
}

/// Marker trait for types that can be safely transmuted from/to raw bytes.
///
/// # Safety
/// Implementors must ensure:
/// - The type is `#[repr(C)]` or `#[repr(transparent)]`
/// - The type has no padding bytes (or padding is always zero-initialized)
/// - The type contains no heap allocations (no Vec, String, Box, etc.)
/// - The type contains no references or raw pointers
/// - All bit patterns are valid for the type
pub unsafe trait Pod: Default + 'static {}

// With the "c-repr" feature enabled for rust_decimal, Decimal is #[repr(C)]
// and can be safely used as a Pod type for zero-copy operations.
// SAFETY: Decimal with c-repr feature is repr(C), 16 bytes, no heap allocations
unsafe impl Pod for Decimal {}

/// Zero-copy cast from bytes to a Pod type reference
///
/// # Safety
/// - The byte slice must be properly aligned for T
/// - The byte slice must have exactly size_of::<T>() bytes
#[inline(always)]
pub unsafe fn from_bytes<T: Pod>(bytes: &[u8]) -> &T {
    debug_assert_eq!(bytes.len(), std::mem::size_of::<T>());
    debug_assert_eq!(bytes.as_ptr() as usize % std::mem::align_of::<T>(), 0);
    &*(bytes.as_ptr() as *const T)
}

/// Zero-copy cast from bytes to a mutable Pod type reference
///
/// # Safety
/// - The byte slice must be properly aligned for T
/// - The byte slice must have exactly size_of::<T>() bytes
#[inline(always)]
pub unsafe fn from_bytes_mut<T: Pod>(bytes: &mut [u8]) -> &mut T {
    debug_assert_eq!(bytes.len(), std::mem::size_of::<T>());
    debug_assert_eq!(bytes.as_ptr() as usize % std::mem::align_of::<T>(), 0);
    &mut *(bytes.as_mut_ptr() as *mut T)
}

/// Zero-copy cast from a Pod type to bytes
#[inline(always)]
pub fn as_bytes<T: Pod>(value: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(value as *const T as *const u8, std::mem::size_of::<T>()) }
}

pub trait UnsafePlugin {
    /// State must implement Pod (plain old data) for zero-copy transmutation
    type State: Pod;

    fn init() -> StrategyConfiguration;
    /// Execute with a mutable reference to the state - true zero-copy
    fn execute(state: &mut Self::State) -> Result<Vec<ExecutionRequest>, String>;
}

#[macro_export]
macro_rules! impl_guest_from_unsafe_plugin {
    ($plugin_type:ty, $name:literal) => {
        impl $crate::bindings::Guest for $plugin_type {
            fn init() -> Vec<u8> {
                let keys = <$plugin_type as $crate::UnsafePlugin>::init();
                borsh::to_vec(&keys).map_err(|err| err.to_string()).unwrap()
            }

            fn exec(mut state: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>), String> {
                type State = <$plugin_type as $crate::UnsafePlugin>::State;

                // If state is empty, allocate and initialize with default
                if state.is_empty() {
                    state = vec![0u8; std::mem::size_of::<State>()];
                    // SAFETY: State implements Pod, zero-initialized is valid, we write default
                    let state_ref: &mut State = unsafe { $crate::from_bytes_mut(&mut state) };
                    *state_ref = State::default();
                }

                if state.len() != std::mem::size_of::<State>() {
                    return Err(format!(
                        "{}: state size mismatch: expected {}, got {}",
                        $name,
                        std::mem::size_of::<State>(),
                        state.len()
                    ));
                }

                // Check alignment - if not aligned, we need to copy (rare case)
                let aligned = state.as_ptr() as usize % std::mem::align_of::<State>() == 0;

                let requests = if aligned {
                    // SAFETY: State implements Pod, alignment verified, size verified
                    let state_ref: &mut State = unsafe { $crate::from_bytes_mut(&mut state) };
                    <$plugin_type as $crate::UnsafePlugin>::execute(state_ref)?
                } else {
                    // Fallback: reallocate with proper alignment (should be rare)
                    let mut aligned_state: State =
                        unsafe { std::ptr::read_unaligned(state.as_ptr() as *const State) };
                    let requests =
                        <$plugin_type as $crate::UnsafePlugin>::execute(&mut aligned_state)?;
                    // Copy back
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            &aligned_state as *const State as *const u8,
                            state.as_mut_ptr(),
                            std::mem::size_of::<State>(),
                        );
                    }
                    requests
                };

                Ok((
                    state, // Return the same Vec, no allocation!
                    borsh::to_vec(&requests).map_err(|err| err.to_string())?,
                ))
            }
        }
    };
}

#[macro_export]
macro_rules! impl_guest_from_plugin {
    ($plugin_type:ty, $name:literal) => {
        impl $crate::bindings::Guest for $plugin_type {
            fn init() -> Vec<u8> {
                let keys = <$plugin_type as $crate::Plugin>::init();
                borsh::to_vec(&keys).map_err(|err| err.to_string()).unwrap()
            }

            fn exec(state: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>), String> {
                let state = if state.is_empty() {
                    <$plugin_type as $crate::Plugin>::State::default()
                } else {
                    let start = std::time::Instant::now();
                    let result = borsh::from_slice(&state).map_err(|err| err.to_string())?;
                    $crate::bindings::record_latency(
                        concat!($name, "_state_deserialize"),
                        start.elapsed().as_nanos() as u64,
                    );
                    result
                };

                let (new_state, requests) = <$plugin_type as $crate::Plugin>::execute(state)?;

                let start = std::time::Instant::now();
                let serialized_state = borsh::to_vec(&new_state).map_err(|err| err.to_string())?;
                $crate::bindings::record_latency(
                    concat!($name, "_state_serialize"),
                    start.elapsed().as_nanos() as u64,
                );

                Ok((
                    serialized_state,
                    borsh::to_vec(&requests).map_err(|err| err.to_string())?,
                ))
            }
        }
    };
}

pub use crate::bindings::{export, Guest};

#[cfg(test)]
mod test {
    use rengine_macros::{sassign, sif};

    #[test]
    #[ignore = "not used"]
    fn test_sif_macro_false() {
        let a = 1;
        let b = 2;
        let c = "bo";
        let d = "bob";

        let result = sif! {
            a > b && c == d,
            {
                // test block
            },
            {
                // else
            }
        };

        assert!(!result.condition);

        assert_eq!(
            result.logs,
            r#"[line 85] cond = false (a = 1) > (b = 2) && (c = "bo") == (d = "bob")"#
        );
        println!("{}", result.logs);
    }

    #[test]
    #[ignore]
    fn test_sassign() {
        let b = 4;
        let c = 8;

        sassign!(let a = b + c);

        assert_eq!(
            __sassign_result.logs,
            r#"[line 96] (a = 12) = (b = 4) + (c = 8)"#
        );

        sassign!(a = a * 2);
        assert_eq!(
            __sassign_result.logs,
            r#"[line 106] (a = 24) = (a = 12) * 2"#
        );
    }
}
