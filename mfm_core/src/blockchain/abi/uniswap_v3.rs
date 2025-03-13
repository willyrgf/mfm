use ethers::{
    abi::{InvalidOutputType, Token, Tokenizable},
    types::{Address, U256},
};

pub const UNISWAP_V3_ROUTER_ABI: &str = r#"[
    {
        "inputs": [
            {
                "components": [
                    {
                        "internalType": "address",
                        "name": "tokenIn",
                        "type": "address"
                    },
                    {
                        "internalType": "address",
                        "name": "tokenOut",
                        "type": "address"
                    },
                    {
                        "internalType": "uint24",
                        "name": "fee",
                        "type": "uint24"
                    },
                    {
                        "internalType": "address",
                        "name": "recipient",
                        "type": "address"
                    },
                    {
                        "internalType": "uint256",
                        "name": "deadline",
                        "type": "uint256"
                    },
                    {
                        "internalType": "uint256",
                        "name": "amountIn",
                        "type": "uint256"
                    },
                    {
                        "internalType": "uint256",
                        "name": "amountOutMinimum",
                        "type": "uint256"
                    },
                    {
                        "internalType": "uint160",
                        "name": "sqrtPriceLimitX96",
                        "type": "uint160"
                    }
                ],
                "internalType": "struct ISwapRouter.ExactInputSingleParams",
                "name": "params",
                "type": "tuple"
            }
        ],
        "name": "exactInputSingle",
        "outputs": [
            {
                "internalType": "uint256",
                "name": "amountOut",
                "type": "uint256"
            }
        ],
        "stateMutability": "payable",
        "type": "function"
    },
    {
        "inputs": [
            {
                "internalType": "address",
                "name": "tokenIn",
                "type": "address"
            },
            {
                "internalType": "address",
                "name": "tokenOut",
                "type": "address"
            },
            {
                "internalType": "uint24",
                "name": "fee",
                "type": "uint24"
            },
            {
                "internalType": "uint256",
                "name": "amountIn",
                "type": "uint256"
            }
        ],
        "name": "quoteExactInputSingle",
        "outputs": [
            {
                "internalType": "uint256",
                "name": "amountOut",
                "type": "uint256"
            }
        ],
        "stateMutability": "nonpayable",
        "type": "function"
    }
]"#;

#[derive(Debug)]
pub struct ExactInputSingleParams {
    pub token_in: Address,
    pub token_out: Address,
    pub fee: u32,
    pub recipient: Address,
    pub deadline: U256,
    pub amount_in: U256,
    pub amount_out_minimum: U256,
    pub sqrt_price_limit_x96: U256,
}

impl Tokenizable for ExactInputSingleParams {
    fn from_token(token: Token) -> Result<Self, InvalidOutputType> {
        if let Token::Tuple(tokens) = token {
            if tokens.len() != 8 {
                return Err(InvalidOutputType(
                    "Expected tuple with 8 elements".to_string(),
                ));
            }
            Ok(Self {
                token_in: Address::from_token(tokens[0].clone())?,
                token_out: Address::from_token(tokens[1].clone())?,
                fee: u32::from_token(tokens[2].clone())?,
                recipient: Address::from_token(tokens[3].clone())?,
                deadline: U256::from_token(tokens[4].clone())?,
                amount_in: U256::from_token(tokens[5].clone())?,
                amount_out_minimum: U256::from_token(tokens[6].clone())?,
                sqrt_price_limit_x96: U256::from_token(tokens[7].clone())?,
            })
        } else {
            Err(InvalidOutputType("Expected tuple".to_string()))
        }
    }

    fn into_token(self) -> Token {
        Token::Tuple(vec![
            self.token_in.into_token(),
            self.token_out.into_token(),
            self.fee.into_token(),
            self.recipient.into_token(),
            self.deadline.into_token(),
            self.amount_in.into_token(),
            self.amount_out_minimum.into_token(),
            self.sqrt_price_limit_x96.into_token(),
        ])
    }
}
