#!/bin/bash
set -e

# Check for required tools
if ! command -v forge &> /dev/null; then
    echo "forge could not be found. Please install foundry."
    exit 1
fi
if ! command -v cast &> /dev/null; then
    echo "cast could not be found. Please install foundry."
    exit 1
fi
if ! command -v cargo &> /dev/null; then
    echo "cargo could not be found. Please install rust."
    exit 1
fi
if ! cargo component --version &> /dev/null; then
    echo "cargo-component could not be found. Please install it: cargo install cargo-component"
    exit 1
fi
if ! command -v wasmtime &> /dev/null; then
    echo "wasmtime could not be found. Please install it."
    exit 1
fi

# Generate bytecode using build script
echo "Generating bytecode..."
cargo build -p evm-types

# Deploy Multicall3
echo "Deploying Multicall3..."
MULTICALL_BYTECODE=$(cat scripts/multicall3.hex)
MULTICALL_OUT=$(cast send --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --rpc-url http://127.0.0.1:8545 --create "$MULTICALL_BYTECODE")
MULTICALL_ADDR=$(echo "$MULTICALL_OUT" | grep "contractAddress" | awk '{print $2}')
echo "Multicall3 deployed to: $MULTICALL_ADDR"

# Deploy MockERC20
echo "Deploying MockERC20..."
ERC20_BYTECODE=$(cat scripts/mock_erc20.hex)
ERC20_OUT=$(cast send --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --rpc-url http://127.0.0.1:8545 --create "$ERC20_BYTECODE")
ERC20_ADDR=$(echo "$ERC20_OUT" | grep "contractAddress" | awk '{print $2}')
echo "MockERC20 deployed to: $ERC20_ADDR"

# Verify address matches strategy
EXPECTED_ADDR="0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
if [ "$ERC20_ADDR" != "$EXPECTED_ADDR" ]; then
    echo "WARNING: Deployed address $ERC20_ADDR does not match strategy expectation $EXPECTED_ADDR"
    echo "You may need to update the strategy code or reset Anvil."
fi

# Check balance of deployer
DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
BALANCE=$(cast call $ERC20_ADDR "balanceOf(address)(uint256)" $DEPLOYER_ADDR --rpc-url http://127.0.0.1:8545)
echo "Deployer ERC20 balance: $BALANCE"

# If balance is 0, try to mint (assuming mint function exists even if not in ABI, or warn)
if [ "$BALANCE" == "0" ]; then
    echo "Balance is 0. Attempting to mint..."
    # Try standard mint signature: mint(address,uint256)
    cast send --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --rpc-url http://127.0.0.1:8545 $ERC20_ADDR "mint(address,uint256)" $DEPLOYER_ADDR 1000000000000000000000 || echo "Mint failed (function might not exist)"
fi

# Build strategies and multicalls
echo "Building strategies and multicalls..."
make

echo ""
echo "Setup complete!"
echo "1. Run the trader:"
echo "   RENGINE_CONFIG=config/test_anvil.toml cargo run --bin trader"
echo ""
echo "2. In another terminal, add the strategy and multicall:"
echo "   ./scripts/strategies.sh add evm_integration_test strategies-wasm/evm_strategy_integration_test.cwasm"
echo "   ./scripts/multicall.sh add anvil evm_integration_test evm_multicalls_wasm/evm_integration_test.cwasm"
echo "   ./scripts/logs.sh add anvil test_logs evm_logs_wasm/test_logs.cwasm"
echo ""
echo "3. Execute the strategy:"
echo "   ./scripts/strategies.sh execute strategies-wasm/evm_strategy_integration_test.cwasm"

