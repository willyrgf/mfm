{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.services.nginx = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    portKeyHttp = lib.mkOption {
      type = t.str;
      default = "http";
    };
    portKeyHttps = lib.mkOption {
      type = t.str;
      default = "https";
    };
  };
}
