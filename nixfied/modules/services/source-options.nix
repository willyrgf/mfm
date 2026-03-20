{ lib }:
let
  t = lib.types;
in
{
  mkSourceSpec =
    {
      withClientPackage ? false,
    }:
    t.submodule {
      options = {
        package = lib.mkOption {
          type = t.nullOr t.package;
          default = null;
        };
        packageAttr = lib.mkOption {
          type = t.nullOr t.str;
          default = null;
        };
        packageFactory = lib.mkOption {
          type = t.nullOr t.path;
          default = null;
        };
      }
      // lib.optionalAttrs withClientPackage {
        clientPackage = lib.mkOption {
          type = t.nullOr t.package;
          default = null;
        };
        clientPackageAttr = lib.mkOption {
          type = t.nullOr t.str;
          default = null;
        };
        clientPackageFactory = lib.mkOption {
          type = t.nullOr t.path;
          default = null;
        };
      };
    };
}
