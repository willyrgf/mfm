pragma solidity 0.8.33;

contract MfmEffectFixture {
    address private immutable owner;
    uint256 public value;

    constructor() {
        owner = msg.sender;
    }

    function configure(uint256 nextValue) external {
        require(msg.sender == owner);
        value = nextValue;
    }
}
