// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IERC20Minimal {
    function balanceOf(address owner) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

// Minimal local-only Aave-like pool used for parity tests.
// Not intended for production.
contract MockAaveV3Pool {
    uint256 public constant WBTC_PRICE_USDC_6 = 100_000e6;
    uint256 public constant COLLATERAL_FACTOR_BPS = 8000;

    address public usdc;
    address public wbtc;
    bool public configured;

    struct Position {
        uint256 suppliedUsdc;
        uint256 collateralWbtc;
        uint256 borrowedUsdc;
    }

    mapping(address => Position) private positions;

    event RuntimeConfigured(address indexed usdc, address indexed wbtc);
    event Supplied(address indexed user, address indexed asset, uint256 amount);
    event Borrowed(address indexed user, address indexed asset, uint256 amount, uint256 rateMode);

    function configureRuntime(address usdc_, address wbtc_) external {
        require(!configured, "already_configured");
        require(usdc_ != address(0), "invalid_usdc");
        require(wbtc_ != address(0), "invalid_wbtc");
        usdc = usdc_;
        wbtc = wbtc_;
        configured = true;
        emit RuntimeConfigured(usdc_, wbtc_);
    }

    function supply(address asset, uint256 amount, address onBehalfOf, uint16) external {
        require(configured, "not_configured");
        require(asset == usdc || asset == wbtc, "unsupported_asset");
        require(onBehalfOf != address(0), "invalid_on_behalf");
        require(amount > 0, "invalid_amount");
        require(
            IERC20Minimal(asset).transferFrom(msg.sender, address(this), amount),
            "transfer_from_failed"
        );

        Position storage p = positions[onBehalfOf];
        if (asset == usdc) {
            p.suppliedUsdc += amount;
        } else {
            p.collateralWbtc += amount;
        }
        emit Supplied(onBehalfOf, asset, amount);
    }

    function borrow(address asset, uint256 amount, uint256 rateMode, uint16, address onBehalfOf) external {
        require(configured, "not_configured");
        require(asset == usdc, "unsupported_borrow_asset");
        require(rateMode == 2, "unsupported_rate_mode");
        require(onBehalfOf != address(0), "invalid_on_behalf");
        require(amount > 0, "invalid_amount");

        Position storage p = positions[onBehalfOf];
        uint256 collateralValueUsdc = (p.collateralWbtc * WBTC_PRICE_USDC_6) / 1e8;
        uint256 maxBorrow = (collateralValueUsdc * COLLATERAL_FACTOR_BPS) / 10_000;
        require(p.borrowedUsdc + amount <= maxBorrow, "borrow_exceeds_ltv");

        uint256 liquidity = IERC20Minimal(usdc).balanceOf(address(this));
        require(liquidity >= amount, "insufficient_liquidity");
        p.borrowedUsdc += amount;

        require(IERC20Minimal(usdc).transfer(onBehalfOf, amount), "transfer_failed");
        emit Borrowed(onBehalfOf, asset, amount, rateMode);
    }

    function suppliedUsdcOf(address user) external view returns (uint256) {
        return positions[user].suppliedUsdc;
    }

    function collateralWbtcOf(address user) external view returns (uint256) {
        return positions[user].collateralWbtc;
    }

    function borrowedUsdcOf(address user) external view returns (uint256) {
        return positions[user].borrowedUsdc;
    }
}
