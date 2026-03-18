use alloy::sol;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    ERC20Mock,
    "abi/mock_erc20.json"
);
