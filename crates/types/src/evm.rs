use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct MulticallPluginConfig {
    pub every_x_block: u64,
}
