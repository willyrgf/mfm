{
  pkgs,
  canonical ? null,
}:
let
  events = import ./events.nix { inherit pkgs; };
  snapshot = import ./snapshot.nix { inherit pkgs; };
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
