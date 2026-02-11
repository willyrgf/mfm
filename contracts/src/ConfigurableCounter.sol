// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract ConfigurableCounter {
    uint256 private value;

    event ValueSet(uint256 value);

    constructor(uint256 initialValue) {
        value = initialValue;
        emit ValueSet(value);
    }

    function setValue(uint256 newValue) external {
        value = newValue;
        emit ValueSet(value);
    }

    function getValue() external view returns (uint256) {
        return value;
    }
}
