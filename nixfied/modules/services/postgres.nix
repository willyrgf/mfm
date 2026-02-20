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
    testDatabase = lib.mkOption {
      type = t.str;
      default = "app_test";
    };
    package = lib.mkOption {
      type = t.nullOr t.str;
      default = null;
    };
    portKey = lib.mkOption {
      type = t.str;
      default = "postgres";
    };
  };
}
