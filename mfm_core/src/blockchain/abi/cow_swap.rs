//! cow swap abi module
//! this module contains the abi for cow swap contracts

/// settlement contract abi for cow swap
pub const SETTLEMENT_ABI: &str = r#"[
    {
        "inputs": [
            {
                "internalType": "contract IGnosisAllowListAuthentication",
                "name": "_authenticator",
                "type": "address"
            },
            {
                "internalType": "contract IBlocklist",
                "name": "_blocklist",
                "type": "address"
            }
        ],
        "stateMutability": "nonpayable",
        "type": "constructor"
    },
    {
        "inputs": [],
        "name": "GPv2Settlement__OrderMustHaveValidSeller",
        "type": "error"
    },
    {
        "inputs": [],
        "name": "InvalidCallData",
        "type": "error"
    },
    {
        "inputs": [],
        "name": "InvalidOrder",
        "type": "error"
    },
    {
        "inputs": [],
        "name": "OnlyOwner",
        "type": "error"
    },
    {
        "inputs": [],
        "name": "OrderNotValid",
        "type": "error"
    },
    {
        "inputs": [],
        "name": "PreSignAlreadySet",
        "type": "error"
    },
    {
        "inputs": [],
        "name": "DOMAIN_SEPARATOR",
        "outputs": [
            {
                "internalType": "bytes32",
                "name": "",
                "type": "bytes32"
            }
        ],
        "stateMutability": "view",
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
                "internalType": "bytes[]",
                "name": "orderUids",
                "type": "bytes[]"
            }
        ],
        "name": "invalidateOrders",
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
        "name": "isOrderValid",
        "outputs": [
            {
                "internalType": "bool",
                "name": "",
                "type": "bool"
            }
        ],
        "stateMutability": "view",
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
        "name": "isValidSignature",
        "outputs": [
            {
                "internalType": "bytes4",
                "name": "",
                "type": "bytes4"
            }
        ],
        "stateMutability": "view",
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
        "name": "preSignature",
        "outputs": [
            {
                "internalType": "bool",
                "name": "signed",
                "type": "bool"
            }
        ],
        "stateMutability": "view",
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
        "name": "setPreSignature",
        "outputs": [],
        "stateMutability": "nonpayable",
        "type": "function"
    }
]"#;

// Constants for order kinds
#[allow(dead_code)]
pub const ORDER_KIND_SELL: &str = "sell";

// Constants for order kinds
#[allow(dead_code)]
pub const ORDER_KIND_BUY: &str = "buy";

// Constants for token balance kinds
#[allow(dead_code)]
pub const BALANCE_ERC20: &str = "erc20";

// Constants for token balance kinds
#[allow(dead_code)]
pub const BALANCE_EXTERNAL: &str = "external";

// Constants for token balance kinds
#[allow(dead_code)]
pub const BALANCE_INTERNAL: &str = "internal";
