{ lib, config, ... }:
{
  options.nixfied.profiles.webapp.enable = lib.mkOption {
    type = lib.types.bool;
    default = false;
  };

  config = lib.mkIf config.nixfied.profiles.webapp.enable {
    nixfied.tasks."dev".tags = lib.mkAfter [ "webapp" ];
    nixfied.tasks."build".tags = lib.mkAfter [ "webapp" ];
  };
}
