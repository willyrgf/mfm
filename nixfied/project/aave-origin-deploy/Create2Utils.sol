// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.0;

library Create2Utils {
  // https://github.com/safe-global/safe-singleton-factory
  address public constant CREATE2_FACTORY = 0x914d7Fec6aaC8cd542e72Bca78B30650d45643d7;

  function _create2Deploy(bytes32 salt, bytes memory bytecode) internal returns (address) {
    bytes32 initcodeHash = keccak256(abi.encodePacked(bytecode));

    if (isContractDeployed(CREATE2_FACTORY) == false) {
      address computed = computeCreate2AddressFrom(address(this), salt, initcodeHash);
      if (isContractDeployed(computed)) {
        return computed;
      }

      address deployedAt;
      assembly {
        deployedAt := create2(0, add(bytecode, 0x20), mload(bytecode), salt)
      }
      require(deployedAt != address(0), "failure at create2 deployment");
      require(deployedAt == computed, "failure at create2 address derivation");
      return deployedAt;
    }

    address computed = computeCreate2Address(salt, initcodeHash);
    if (isContractDeployed(computed)) {
      return computed;
    }

    bytes memory creationBytecode = abi.encodePacked(salt, bytecode);
    (bool success, bytes memory returnData) = CREATE2_FACTORY.call(creationBytecode);
    require(success, "failure at create2 deployment");
    address deployedAt = address(uint160(bytes20(returnData)));
    require(deployedAt == computed, "failure at create2 address derivation");
    return deployedAt;
  }

  function isContractDeployed(address _addr) internal view returns (bool isContract) {
    return (_addr.code.length > 0);
  }

  function computeCreate2Address(bytes32 salt, bytes32 initcodeHash) internal view returns (address) {
    if (isContractDeployed(CREATE2_FACTORY)) {
      return computeCreate2AddressFrom(CREATE2_FACTORY, salt, initcodeHash);
    }
    return computeCreate2AddressFrom(address(this), salt, initcodeHash);
  }

  function computeCreate2Address(bytes32 salt, bytes memory bytecode) internal view returns (address) {
    return computeCreate2Address(salt, keccak256(abi.encodePacked(bytecode)));
  }

  function computeCreate2AddressFrom(address deployer, bytes32 salt, bytes32 initcodeHash) internal pure returns (address) {
    return addressFromLast20Bytes(keccak256(abi.encodePacked(bytes1(0xff), deployer, salt, initcodeHash)));
  }

  function addressFromLast20Bytes(bytes32 bytesValue) internal pure returns (address) {
    return address(uint160(uint256(bytesValue)));
  }
}
