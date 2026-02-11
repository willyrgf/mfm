// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract ConfigurableCounter {
    address public immutable owner;
    uint256 private value;

    event ValueSet(uint256 value);

    constructor(uint256 initialValue) {
        owner = msg.sender;
        value = initialValue;
        emit ValueSet(initialValue);
    }

    function setValue(uint256 newValue) external {
        require(msg.sender == owner, "not owner");
        value = newValue;
        emit ValueSet(newValue);
    }

    function getValue() external view returns (uint256) {
        return value;
    }
}
