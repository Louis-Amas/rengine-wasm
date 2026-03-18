use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{bindings::IMulticall3, Provider},
    rpc::types::TransactionRequest,
};
use anyhow::{Context, Result};
use evm_types::erc20::ERC20Mock;

pub async fn deploy_multicall3<P>(provider: &P) -> Result<Address>
where
    P: Provider + Send + Sync,
{
    // Take the runtime bytecode straight from the generated binding
    let bytecode: Bytes = IMulticall3::BYTECODE.clone();

    // Create a contract-creation tx (no constructor args for Multicall3)
    let tx = TransactionRequest::default().with_deploy_code(bytecode);

    // Send & wait for receipt
    let receipt = provider
        .send_transaction(tx)
        .await
        .context("failed to send deploy tx")?
        .get_receipt()
        .await
        .context("failed to fetch receipt")?;

    // Get the deployed address
    let addr = receipt
        .contract_address
        .context("receipt missing contract_address")?;

    Ok(addr)
}

pub async fn deploy_erc20_mock<P>(provider: &P) -> Result<Address>
where
    P: Provider + Send + Sync,
{
    // 1) init code = bytecode only
    let init_code: Bytes = ERC20Mock::BYTECODE.clone();

    // 2) create contract-creation tx
    let tx = TransactionRequest::default().with_deploy_code(init_code);

    // 3) send + wait
    let receipt = provider
        .send_transaction(tx)
        .await
        .context("failed to send ERC20Mock deploy tx")?
        .get_receipt()
        .await
        .context("failed to fetch ERC20Mock receipt")?;

    // 4) deployed address
    let addr = receipt
        .contract_address
        .context("receipt missing contract_address for ERC20Mock")?;

    Ok(addr)
}
