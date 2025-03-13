use ethers::prelude::*;

// PoolManager ABI
abigen!(
    IPoolManager,
    r#"[
        {
            "inputs": [
                {
                    "components": [
                        {
                            "internalType": "address",
                            "name": "currency0",
                            "type": "address"
                        },
                        {
                            "internalType": "address",
                            "name": "currency1",
                            "type": "address"
                        },
                        {
                            "internalType": "uint24",
                            "name": "fee",
                            "type": "uint24"
                        },
                        {
                            "internalType": "int24",
                            "name": "tickSpacing",
                            "type": "int24"
                        },
                        {
                            "internalType": "address",
                            "name": "hooks",
                            "type": "address"
                        }
                    ],
                    "internalType": "struct PoolKey",
                    "name": "key",
                    "type": "tuple"
                },
                {
                    "components": [
                        {
                            "internalType": "bool",
                            "name": "zeroForOne",
                            "type": "bool"
                        },
                        {
                            "internalType": "int256",
                            "name": "amountSpecified",
                            "type": "int256"
                        },
                        {
                            "internalType": "uint160",
                            "name": "sqrtPriceLimitX96",
                            "type": "uint160"
                        }
                    ],
                    "internalType": "struct SwapParams",
                    "name": "params",
                    "type": "tuple"
                }
            ],
            "name": "swap",
            "outputs": [
                {
                    "components": [
                        {
                            "internalType": "int256",
                            "name": "amount0",
                            "type": "int256"
                        },
                        {
                            "internalType": "int256",
                            "name": "amount1",
                            "type": "int256"
                        }
                    ],
                    "internalType": "struct BalanceDelta",
                    "name": "delta",
                    "type": "tuple"
                }
            ],
            "stateMutability": "nonpayable",
            "type": "function"
        },
        {
            "inputs": [
                {
                    "internalType": "address",
                    "name": "currency",
                    "type": "address"
                },
                {
                    "internalType": "address",
                    "name": "to",
                    "type": "address"
                },
                {
                    "internalType": "uint256",
                    "name": "amount",
                    "type": "uint256"
                }
            ],
            "name": "take",
            "outputs": [],
            "stateMutability": "nonpayable",
            "type": "function"
        },
        {
            "inputs": [
                {
                    "internalType": "address",
                    "name": "currency",
                    "type": "address"
                }
            ],
            "name": "settle",
            "outputs": [],
            "stateMutability": "payable",
            "type": "function"
        }
    ]"#
);
