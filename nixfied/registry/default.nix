{
  pkgs,
}:
let
  events = import ./events.nix { inherit pkgs; };
  snapshot = import ./snapshot.nix { };
  replay = import ./replay.nix { inherit pkgs; };
in
{
  inherit
    events
    snapshot
    replay
    ;

  mkReplayApp = replay.mkReplayTool { };
}
