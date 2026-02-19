{ lib, config, ... }:
{
  options.nixfied.profiles.eth.enable = lib.mkOption {
    type = lib.types.bool;
    default = false;
  };

  config = lib.mkIf config.nixfied.profiles.eth.enable {
    nixfied.tasks."dev".tags = lib.mkAfter [ "eth" ];
    nixfied.tasks."ci".tags = lib.mkAfter [ "eth" ];
  };
}
