{
  lib,
  canonical,
}:
{
  projectRoot,
  resolvedIdentity,
  baseStatePolicy,
  serviceCatalog,
  resolved,
}:
let
  statePolicyLib = import ./state-policy-lib.nix { inherit lib; };
  rawServiceSets = resolved.serviceSets or { };
  names = builtins.sort builtins.lessThan (builtins.attrNames rawServiceSets);
  enabledServiceNames = builtins.sort builtins.lessThan (
    map
      (
        serviceId:
        let
          service = serviceCatalog.${serviceId};
        in
        service.name
      )
      (
        builtins.filter (serviceId: serviceCatalog.${serviceId}.enable or false) (
          builtins.attrNames serviceCatalog
        )
      )
  );
  knownServiceNames = builtins.sort builtins.lessThan (
    map (serviceId: serviceCatalog.${serviceId}.name) (builtins.attrNames serviceCatalog)
  );

  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);

  normalizeServiceSet =
    name:
    let
      raw = rawServiceSets.${name};
      requiredServices = uniqueSorted (raw.services.required or [ ]);
      optionalServices = uniqueSorted (raw.services.optional or [ ]);
      unknownServices = builtins.filter (serviceName: !(builtins.elem serviceName knownServiceNames)) (
        requiredServices ++ optionalServices
      );
      disabledServices = builtins.filter (serviceName: !(builtins.elem serviceName enabledServiceNames)) (
        requiredServices ++ optionalServices
      );
      overlappingServices = builtins.filter (
        serviceName: builtins.elem serviceName optionalServices
      ) requiredServices;
      serviceSetId = if (raw.id or "") == "" then "service-set.${name}" else raw.id;
      effectivePolicy =
        if (raw.state.policy or null) == null then
          baseStatePolicy
        else
          statePolicyLib.compilePolicy {
            inherit projectRoot;
            identity = resolvedIdentity;
            policy = raw.state.policy;
          };
    in
    if unknownServices != [ ] then
      throw "nixfied service sets: '${name}' references unknown services: ${builtins.concatStringsSep ", " unknownServices}"
    else if disabledServices != [ ] then
      throw "nixfied service sets: '${name}' references disabled services: ${builtins.concatStringsSep ", " disabledServices}"
    else if overlappingServices != [ ] then
      throw "nixfied service sets: '${name}' lists services as both required and optional: ${builtins.concatStringsSep ", " overlappingServices}"
    else
      canonical.canonicalize {
        id = serviceSetId;
        name = name;
        summary = raw.summary or "Service set ${name}";
        description = raw.description or "";
        services = {
          required = requiredServices;
          optional = optionalServices;
          all = uniqueSorted (requiredServices ++ optionalServices);
        };
        defaultOperation = raw.defaultOperation or "health";
        state = {
          policy = effectivePolicy;
        };
        export = {
          defaultFormat = ((raw.export or { }).defaultFormat or "json");
        };
        failureLogs = {
          capture = ((raw.failureLogs or { }).capture or true);
          tailLines = ((raw.failureLogs or { }).tailLines or 40);
        };
        ownerFile = if (raw.ownerFile or null) == null || raw.ownerFile == "" then null else raw.ownerFile;
      };

  addServiceSet =
    acc: name:
    let
      serviceSet = normalizeServiceSet name;
      serviceSetId = serviceSet.id;
    in
    if builtins.hasAttr serviceSetId acc then
      throw "nixfied service sets: duplicate service set id '${serviceSetId}'"
    else
      acc // { ${serviceSetId} = serviceSet; };
in
builtins.foldl' addServiceSet { } names
