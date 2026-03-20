{ lib }:
{ resolved }:
let
  services = resolved.services or { };
  excludedServices = resolved.graph.excludedServices or [ ];
  names = builtins.sort builtins.lessThan (
    builtins.filter (name: !(builtins.elem name excludedServices)) (builtins.attrNames services)
  );

  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);

  sourceKeysFor =
    serviceCfg:
    uniqueSorted ((serviceCfg.sourceKeys or [ ]) ++ builtins.attrNames (serviceCfg.sources or { }));

  probeModesFor =
    serviceCfg:
    uniqueSorted (
      builtins.attrNames (serviceCfg.probes or { })
      ++ builtins.attrNames (serviceCfg.operationProbes or { })
    );
in
builtins.listToAttrs (
  map (
    name:
    let
      serviceCfg = services.${name};
    in
    {
      name = "service.${name}";
      value = {
        id = "service.${name}";
        name = name;
        enable = serviceCfg.enable or false;
        config = {
          dataDirName = serviceCfg.dataDirName or name;
          defaultSource = serviceCfg.defaultSource or "";
          sourceKeys = sourceKeysFor serviceCfg;
          probeModes = probeModesFor serviceCfg;
        };
      };
    }
  ) names
)
