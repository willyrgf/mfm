use ethers::prelude::*;

// Factory ABI
abigen!(
    IUniswapV3Factory,
    r#"[
        function createPool(address tokenA, address tokenB, uint24 fee) external returns (address pool)
        function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool)
        function owner() external view returns (address)
    ]"#
);

// Pool ABI
abigen!(
    IUniswapV3Pool,
    r#"[
        function token0() external view returns (address)
        function token1() external view returns (address)
        function fee() external view returns (uint24)
        function tickSpacing() external view returns (int24)
        function liquidity() external view returns (uint128)
        function slot0() external view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)
        function swap(address recipient, bool zeroForOne, int256 amountSpecified, uint160 sqrtPriceLimitX96, bytes calldata data) external returns (int256 amount0, int256 amount1)
    ]"#
);

// SwapRouter ABI
abigen!(
    ISwapRouter,
    r#"[{
        "inputs": [{
            "components": [{
                "internalType": "address",
                "name": "tokenIn",
                "type": "address"
            }, {
                "internalType": "address",
                "name": "tokenOut",
                "type": "address"
            }, {
                "internalType": "uint24",
                "name": "fee",
                "type": "uint24"
            }, {
                "internalType": "address",
                "name": "recipient",
                "type": "address"
            }, {
                "internalType": "uint256",
                "name": "deadline",
                "type": "uint256"
            }, {
                "internalType": "uint256",
                "name": "amountIn",
                "type": "uint256"
            }, {
                "internalType": "uint256",
                "name": "amountOutMinimum",
                "type": "uint256"
            }, {
                "internalType": "uint160",
                "name": "sqrtPriceLimitX96",
                "type": "uint160"
            }],
            "internalType": "struct ISwapRouter.ExactInputSingleParams",
            "name": "params",
            "type": "tuple"
        }],
        "name": "exactInputSingle",
        "outputs": [{
            "internalType": "uint256",
            "name": "amountOut",
            "type": "uint256"
        }],
        "stateMutability": "payable",
        "type": "function"
    }]"#
);

// Quoter ABI
abigen!(
    IQuoter,
    r#"[
        function quoteExactInputSingle(address tokenIn, address tokenOut, uint24 fee, uint256 amountIn, uint160 sqrtPriceLimitX96) external returns (uint256 amountOut)
    ]"#
);
