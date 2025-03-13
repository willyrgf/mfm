use ethers::abi::Abi;
use lazy_static::lazy_static;
use serde_json::json;

lazy_static! {
    // GPv2Settlement Contract ABI
    pub static ref SETTLEMENT_ABI: Abi = serde_json::from_value(json!({
        "abi": [
            {
                "inputs": [
                    {
                        "internalType": "IERC20[]",
                        "name": "tokens",
                        "type": "address[]"
                    },
                    {
                        "internalType": "uint256[]",
                        "name": "clearingPrices",
                        "type": "uint256[]"
                    },
                    {
                        "components": [
                            {
                                "components": [
                                    {
                                        "internalType": "address",
                                        "name": "sellToken",
                                        "type": "address"
                                    },
                                    {
                                        "internalType": "address",
                                        "name": "buyToken",
                                        "type": "address"
                                    },
                                    {
                                        "internalType": "address",
                                        "name": "receiver",
                                        "type": "address"
                                    },
                                    {
                                        "internalType": "uint256",
                                        "name": "sellAmount",
                                        "type": "uint256"
                                    },
                                    {
                                        "internalType": "uint256",
                                        "name": "buyAmount",
                                        "type": "uint256"
                                    },
                                    {
                                        "internalType": "uint32",
                                        "name": "validTo",
                                        "type": "uint32"
                                    },
                                    {
                                        "internalType": "bytes32",
                                        "name": "appData",
                                        "type": "bytes32"
                                    },
                                    {
                                        "internalType": "uint256",
                                        "name": "feeAmount",
                                        "type": "uint256"
                                    },
                                    {
                                        "internalType": "bytes32",
                                        "name": "kind",
                                        "type": "bytes32"
                                    },
                                    {
                                        "internalType": "bool",
                                        "name": "partiallyFillable",
                                        "type": "bool"
                                    },
                                    {
                                        "internalType": "bytes32",
                                        "name": "sellTokenBalance",
                                        "type": "bytes32"
                                    },
                                    {
                                        "internalType": "bytes32",
                                        "name": "buyTokenBalance",
                                        "type": "bytes32"
                                    }
                                ],
                                "internalType": "struct GPv2Order.Data",
                                "name": "order",
                                "type": "tuple"
                            },
                            {
                                "internalType": "bytes",
                                "name": "signature",
                                "type": "bytes"
                            },
                            {
                                "internalType": "uint256",
                                "name": "executedAmount",
                                "type": "uint256"
                            }
                        ],
                        "internalType": "struct GPv2Trade.Data[]",
                        "name": "trades",
                        "type": "tuple[]"
                    },
                    {
                        "components": [
                            {
                                "internalType": "address",
                                "name": "target",
                                "type": "address"
                            },
                            {
                                "internalType": "uint256",
                                "name": "value",
                                "type": "uint256"
                            },
                            {
                                "internalType": "bytes",
                                "name": "callData",
                                "type": "bytes"
                            }
                        ],
                        "internalType": "struct GPv2Interaction.Data[][3]",
                        "name": "interactions",
                        "type": "tuple[][3]"
                    }
                ],
                "name": "settle",
                "outputs": [],
                "stateMutability": "nonpayable",
                "type": "function"
            },
            {
                "inputs": [
                    {
                        "internalType": "bytes",
                        "name": "orderUid",
                        "type": "bytes"
                    }
                ],
                "name": "invalidateOrder",
                "outputs": [],
                "stateMutability": "nonpayable",
                "type": "function"
            },
            {
                "inputs": [
                    {
                        "internalType": "bytes",
                        "name": "orderUid",
                        "type": "bytes"
                    },
                    {
                        "internalType": "bool",
                        "name": "signed",
                        "type": "bool"
                    }
                ],
                "name": "setPreSignature",
                "outputs": [],
                "stateMutability": "nonpayable",
                "type": "function"
            }
        ]
    })).unwrap();

    // Order struct for type definitions
    pub static ref ORDER_STRUCT: serde_json::Value = json!({
        "components": [
            {
                "internalType": "address",
                "name": "sellToken",
                "type": "address"
            },
            {
                "internalType": "address",
                "name": "buyToken",
                "type": "address"
            },
            {
                "internalType": "address",
                "name": "receiver",
                "type": "address"
            },
            {
                "internalType": "uint256",
                "name": "sellAmount",
                "type": "uint256"
            },
            {
                "internalType": "uint256",
                "name": "buyAmount",
                "type": "uint256"
            },
            {
                "internalType": "uint32",
                "name": "validTo",
                "type": "uint32"
            },
            {
                "internalType": "bytes32",
                "name": "appData",
                "type": "bytes32"
            },
            {
                "internalType": "uint256",
                "name": "feeAmount",
                "type": "uint256"
            },
            {
                "internalType": "bytes32",
                "name": "kind",
                "type": "bytes32"
            },
            {
                "internalType": "bool",
                "name": "partiallyFillable",
                "type": "bool"
            },
            {
                "internalType": "bytes32",
                "name": "sellTokenBalance",
                "type": "bytes32"
            },
            {
                "internalType": "bytes32",
                "name": "buyTokenBalance",
                "type": "bytes32"
            }
        ],
        "name": "order",
        "type": "tuple"
    });
}

// Constants for order kinds
pub const ORDER_KIND_SELL: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";
pub const ORDER_KIND_BUY: &str =
    "0xf3b277728b3fee749481eb3e0b3b48980dbbab78658fc419025cb16eee346775";

// Constants for token balance kinds
pub const BALANCE_ERC20: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";
pub const BALANCE_EXTERNAL: &str =
    "0x1000000000000000000000000000000000000000000000000000000000000000";
pub const BALANCE_INTERNAL: &str =
    "0x2000000000000000000000000000000000000000000000000000000000000000";
