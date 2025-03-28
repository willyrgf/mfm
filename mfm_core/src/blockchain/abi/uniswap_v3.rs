use alloy_sol_types::sol;

// Factory interface
sol! {
    #[derive(Debug)]
    interface IUniswapV3Factory {
        function createPool(address tokenA, address tokenB, uint24 fee) external returns (address pool);
        function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool);
        function owner() external view returns (address);
    }
}

// Pool interface
sol! {
    #[derive(Debug)]
    interface IUniswapV3Pool {
        function token0() external view returns (address);
        function token1() external view returns (address);
        function fee() external view returns (uint24);
        function tickSpacing() external view returns (int24);
        function liquidity() external view returns (uint128);

        struct Slot0 {
            uint160 sqrtPriceX96;
            int24 tick;
            uint16 observationIndex;
            uint16 observationCardinality;
            uint16 observationCardinalityNext;
            uint8 feeProtocol;
            bool unlocked;
        }

        function slot0() external view returns (Slot0 memory);
        function swap(address recipient, bool zeroForOne, int256 amountSpecified, uint160 sqrtPriceLimitX96, bytes calldata data) external returns (int256 amount0, int256 amount1);
    }
}

// SwapRouter interface
sol! {
    #[derive(Debug)]
    interface ISwapRouter {
        struct ExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint24 fee;
            address recipient;
            uint256 deadline;
            uint256 amountIn;
            uint256 amountOutMinimum;
            uint160 sqrtPriceLimitX96;
        }

        function exactInputSingle(ExactInputSingleParams calldata params) external payable returns (uint256 amountOut);
    }
}

// Quoter interface
sol! {
    #[derive(Debug)]
    interface IQuoter {
        function quoteExactInputSingle(
            address tokenIn,
            address tokenOut,
            uint24 fee,
            uint256 amountIn,
            uint160 sqrtPriceLimitX96
        ) external returns (uint256 amountOut);
    }
}

// WETH interface
sol! {
    #[derive(Debug)]
    interface IWETH9 {
        function deposit() external payable;
        function withdraw(uint256 amount) external;
        function balanceOf(address account) external view returns (uint256);
    }
}

// ERC20 interface
sol! {
    #[derive(Debug)]
    interface IERC20 {
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function decimals() external view returns (uint8);
        function symbol() external view returns (string memory);
    }
}
