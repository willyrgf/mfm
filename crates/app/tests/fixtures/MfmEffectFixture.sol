pragma solidity 0.8.33;

contract MfmEffectFixture {
    address public owner;
    uint256 public value;

    event Configured(uint256 value);

    constructor(uint256 initialValue) {
        owner = msg.sender;
        value = initialValue;
    }

    function configure(uint256 nextValue) external {
        require(msg.sender == owner);
        value = nextValue;
        emit Configured(nextValue);
    }
}
