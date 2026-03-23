{ lib }:
{ projectRoot, resolved }:
let
  statePolicyLib = import ./state-policy-lib.nix { inherit lib; };
in
statePolicyLib.compilePolicy {
  inherit projectRoot;
  identity = resolved.identity;
  policy = resolved.state.policy;
}
