{
  pkgs,
  canonical,
  bundle,
}:
let
  lib = pkgs.lib;

  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);
  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);
  modeNames = [
    "default"
    "resolution"
    "execution"
    "closure"
  ];

  mkEnvelope =
    payload:
    canonical.canonicalize {
      kind = "introspection-response";
      version = 1;
      inherit payload;
    };

  queryPayloadFor = nodeId: mode: bundle.nodeViews.${nodeId}.jsonByMode.${mode};

  reversePayloadFor = targetNodeId: mode: bundle.reverseViews.${targetNodeId}.jsonByMode.${mode};

  whyPayloadFor =
    sourceNodeId: targetNodeId: mode:
    let
      view = queryPayloadFor sourceNodeId mode;
      reverseView = bundle.reverseViews.${targetNodeId};
      closureView = if (view ? closure) && view.closure != null then view.closure else { };
    in
    if mode == "resolution" || mode == "execution" then
      view
    else
      canonical.canonicalize (
        view
        // {
          closure = {
            target = reverseView.target;
            summary = closureView.summary or null;
            reasonChains = reverseView.reasonChainsBySource.${sourceNodeId} or [ ];
          };
          reverse = null;
        }
      );

  mkTextSpec = path: content: {
    inherit
      path
      content
      ;
  };

  mkJsonSpec = path: value: mkTextSpec path (builtins.toJSON value);

  entryTsv =
    entry:
    builtins.concatStringsSep "\t" [
      entry.nodeId
      entry.sourceKind
      entry.selector
    ];

  queryNodeIds = sortNames bundle.nodeViews;
  reverseTargetNodeIds = sortNames bundle.reverseViews;

  explicitSpecs = lib.mapAttrsToList (
    token: entry: mkTextSpec "resolution/explicit/${token}.tsv" (entryTsv entry)
  ) bundle.resolutionIndex.explicit;

  bareAnySpecs = lib.mapAttrsToList (
    token: entry: mkTextSpec "resolution/bare/any/${token}.tsv" (entryTsv entry)
  ) bundle.resolutionIndex.bare.any;

  bareByKindSpecs = builtins.concatLists (
    map (
      kind:
      lib.mapAttrsToList (
        token: entry: mkTextSpec "resolution/bare/by-kind/${kind}/${token}.tsv" (entryTsv entry)
      ) bundle.resolutionIndex.bare.byKind.${kind}
    ) (sortNames bundle.resolutionIndex.bare.byKind)
  );

  ambiguousAnySpecs = lib.mapAttrsToList (
    token: labels:
    mkTextSpec "resolution/ambiguous/any/${token}.txt" (builtins.concatStringsSep "\n" labels)
  ) bundle.resolutionIndex.ambiguousBare.any;

  ambiguousByKindSpecs = builtins.concatLists (
    map (
      kind:
      lib.mapAttrsToList (
        token: labels:
        mkTextSpec "resolution/ambiguous/by-kind/${kind}/${token}.txt" (
          builtins.concatStringsSep "\n" labels
        )
      ) bundle.resolutionIndex.ambiguousBare.byKind.${kind}
    ) (sortNames bundle.resolutionIndex.ambiguousBare.byKind)
  );

  queryJsonSpecs = builtins.concatLists (
    map (
      nodeId:
      map (
        mode:
        mkJsonSpec "views/query/json/${mode}/${nodeId}.json" (mkEnvelope (queryPayloadFor nodeId mode))
      ) modeNames
    ) queryNodeIds
  );

  queryHumanSpecs = builtins.concatLists (
    map (
      nodeId:
      map (
        mode:
        mkTextSpec "views/query/human/${mode}/${nodeId}.txt" (
          builtins.concatStringsSep "\n" (bundle.nodeViews.${nodeId}.humanByMode.${mode} or [ ])
        )
      ) modeNames
    ) queryNodeIds
  );

  reverseJsonSpecs = builtins.concatLists (
    map (
      targetNodeId:
      map (
        mode:
        mkJsonSpec "views/reverse/json/${mode}/${targetNodeId}.json" (
          mkEnvelope (reversePayloadFor targetNodeId mode)
        )
      ) modeNames
    ) reverseTargetNodeIds
  );

  reverseHumanSpecs = builtins.concatLists (
    map (
      targetNodeId:
      map (
        mode:
        mkTextSpec "views/reverse/human/${mode}/${targetNodeId}.txt" (
          builtins.concatStringsSep "\n" (bundle.reverseViews.${targetNodeId}.humanByMode.${mode} or [ ])
        )
      ) modeNames
    ) reverseTargetNodeIds
  );

  whyJsonSpecs = builtins.concatLists (
    map (
      targetNodeId:
      let
        sourceNodeIds = queryNodeIds;
      in
      builtins.concatLists (
        map (
          sourceNodeId:
          map (
            mode:
            mkJsonSpec "views/why/json/${mode}/${sourceNodeId}/${targetNodeId}.json" (
              mkEnvelope (whyPayloadFor sourceNodeId targetNodeId mode)
            )
          ) modeNames
        ) sourceNodeIds
      )
    ) reverseTargetNodeIds
  );

  whyHumanSpecs = builtins.concatLists (
    map (
      targetNodeId:
      let
        reverseView = bundle.reverseViews.${targetNodeId};
        sourceNodeIds = queryNodeIds;
      in
      builtins.concatLists (
        map (
          sourceNodeId:
          map (
            mode:
            mkTextSpec "views/why/human/${mode}/${sourceNodeId}/${targetNodeId}.txt" (
              builtins.concatStringsSep "\n" (
                reverseView.whyLinesBySource.${sourceNodeId}
                  or [ "WARN: no reason chains from ${sourceNodeId} to ${targetNodeId}" ]
              )
            )
          ) modeNames
        ) sourceNodeIds
      )
    ) reverseTargetNodeIds
  );

  staticSpecs = [
    (mkTextSpec "diagnostics/human.txt" (builtins.concatStringsSep "\n" bundle.diagnosticsHumanLines))
    (mkTextSpec "valid-kinds.txt" (builtins.concatStringsSep "\n" bundle.validKinds))
  ];

  fileSpecs =
    staticSpecs
    ++ explicitSpecs
    ++ bareAnySpecs
    ++ bareByKindSpecs
    ++ ambiguousAnySpecs
    ++ ambiguousByKindSpecs
    ++ queryJsonSpecs
    ++ queryHumanSpecs
    ++ reverseJsonSpecs
    ++ reverseHumanSpecs
    ++ whyJsonSpecs
    ++ whyHumanSpecs;

  parentDirs = builtins.filter (dir: dir != ".") (
    uniqueSorted (map (spec: builtins.dirOf spec.path) fileSpecs)
  );

  mkdirCommands = builtins.concatStringsSep "\n" (map (dir: ''mkdir -p "$out/${dir}"'') parentDirs);

  writeCommands = builtins.concatStringsSep "\n" (
    map (spec: ''
      printf '%s\n' ${lib.escapeShellArg spec.content} > "$out/${spec.path}"
    '') fileSpecs
  );
in
pkgs.runCommand "nixfied-introspection-assets" { } ''
  mkdir -p "$out"
  ${mkdirCommands}
  ${writeCommands}
''
