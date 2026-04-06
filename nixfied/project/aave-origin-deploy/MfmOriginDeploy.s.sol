// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.0;

import "forge-std/Script.sol";
import "forge-std/StdJson.sol";

import {AaveV3BatchOrchestration} from "../src/deployments/projects/aave-v3-batched/AaveV3BatchOrchestration.sol";
import {DefaultMarketInput} from "../src/deployments/inputs/DefaultMarketInput.sol";
import {MarketReport, Roles, MarketConfig, DeployFlags} from "../src/deployments/interfaces/IMarketReportTypes.sol";
import {IAaveV3ConfigEngine} from "../src/contracts/extensions/v3-config-engine/AaveV3ConfigEngine.sol";
import {AaveV3TestListing} from "../tests/mocks/AaveV3TestListing.sol";
import {ACLManager} from "../src/contracts/protocol/configuration/ACLManager.sol";
import {WETH9} from "../src/contracts/dependencies/weth/WETH9.sol";
import {TestnetERC20} from "../src/contracts/mocks/testnet-helpers/TestnetERC20.sol";
import {IPool} from "../src/contracts/interfaces/IPool.sol";
import {DataTypes} from "../src/contracts/protocol/libraries/types/DataTypes.sol";

contract MfmOriginDeploy is Script, DefaultMarketInput {
  using stdJson for string;

  function run() external {
    uint256 deployerPrivateKey = vm.envUint("MFM_AAVE_V3_PARITY_DEPLOYER_PRIVATE_KEY");
    address deployer = vm.addr(deployerPrivateKey);
    address supplier = vm.envAddress("MFM_AAVE_V3_ORIGIN_SUPPLIER");
    address borrower = vm.envAddress("MFM_AAVE_V3_ORIGIN_BORROWER");
    uint256 usdcSupplyAmount = vm.envUint("MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT");
    uint256 wbtcCollateralAmount = vm.envUint("MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT");

    (Roles memory roles, MarketConfig memory config, DeployFlags memory flags, MarketReport memory report) =
      _getMarketInput(deployer);
    roles.marketOwner = deployer;
    roles.poolAdmin = deployer;
    roles.emergencyAdmin = deployer;

    vm.startBroadcast(deployerPrivateKey);

    address weth = address(new WETH9());
    config.wrappedNativeToken = weth;

    report = AaveV3BatchOrchestration.deployAaveV3(deployer, roles, config, flags, report);

    AaveV3TestListing listing = new AaveV3TestListing(
      IAaveV3ConfigEngine(report.configEngine),
      roles.poolAdmin,
      weth,
      report
    );

    ACLManager manager = ACLManager(report.aclManager);
    manager.addPoolAdmin(address(listing));
    listing.execute();

    address usdc = listing.USDX_ADDRESS();
    address wbtc = listing.WBTC_ADDRESS();

    TestnetERC20(usdc).mint(supplier, usdcSupplyAmount);
    TestnetERC20(wbtc).mint(borrower, wbtcCollateralAmount);

    DataTypes.ReserveDataLegacy memory usdcReserve = IPool(report.poolProxy).getReserveData(usdc);
    DataTypes.ReserveDataLegacy memory wbtcReserve = IPool(report.poolProxy).getReserveData(wbtc);

    vm.stopBroadcast();

    string memory root = "mfm_origin_deploy";
    vm.serializeString(root, "kind", "aave_v3_origin_deploy_output_v1");
    vm.serializeAddress(root, "pool", report.poolProxy);
    vm.serializeAddress(root, "usdc", usdc);
    vm.serializeAddress(root, "wbtc", wbtc);
    vm.serializeAddress(root, "usdcAToken", usdcReserve.aTokenAddress);
    vm.serializeAddress(root, "wbtcAToken", wbtcReserve.aTokenAddress);
    vm.serializeAddress(root, "usdcVariableDebtToken", usdcReserve.variableDebtTokenAddress);
    string memory out = vm.serializeAddress(root, "wbtcVariableDebtToken", wbtcReserve.variableDebtTokenAddress);
    vm.writeJson(out, "./reports/mfm-origin-deploy.json");
  }
}
