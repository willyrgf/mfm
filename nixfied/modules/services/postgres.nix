{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.services.postgres = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    database = lib.mkOption {
      type = t.str;
      default = "app";
    };
    portKey = lib.mkOption {
      type = t.str;
      default = "postgres";
    };
    sourceKeys = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
    };
    defaultSource = lib.mkOption {
      type = t.str;
      default = "";
    };
  };
}
