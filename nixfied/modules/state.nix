{ lib, ... }:
let
  statePolicyOptions = import ./lib/state-policy-options.nix { inherit lib; };
in
{
  options.nixfied.state = {
    policy = statePolicyOptions;
  };
}
