{
  lib,
  canonical,
}:
{
  introspectionGraph,
  maxWhyDepth ? 12,
}:
let
  kindNames = [
    "app"
    "package"
    "service"
    "service-set"
    "task"
    "workflow"
  ];

  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);
  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);
  nonEmptyStrings = values: builtins.filter (value: value != null && value != "") values;
  joinCsv = values: builtins.concatStringsSep "," values;
  stringOrEmpty = value: if value == null then "" else value;
  mapIndexed =
    f: values:
    builtins.genList (index: f index (builtins.elemAt values index)) (builtins.length values);

  nodeIds = sortNames introspectionGraph.nodes;
  nonExecutionNodeIds = builtins.filter (
    nodeId: introspectionGraph.nodes.${nodeId}.kind != "execution"
  ) nodeIds;

  diagnostics = canonical.canonicalize {
    localOverridesActive = introspectionGraph.localOverrides.active or false;
    localOverrideCount = introspectionGraph.localOverrides.count or 0;
    legacyLocalDefault =
      introspectionGraph.legacyLocalDefault or {
        path = "nixfied/local/default.nix";
        present = false;
        customized = false;
        active = false;
        status = "missing";
        message = "legacy local/default.nix is absent";
      };
    policyId = introspectionGraph.state.policyId or "";
    policyKind = introspectionGraph.state.policyKind or "";
    policySource = introspectionGraph.state.policySource or "";
    ownerScope = introspectionGraph.state.ownerScope or "";
    discoveryScope = introspectionGraph.state.discoveryScope or "";
    workspaceMarkerPresent = introspectionGraph.state.workspaceMarkerPresent or false;
    runtimeBase = introspectionGraph.state.runtimeBase or "";
    registryRoot = introspectionGraph.state.registryRoot or "";
    artifactsRoot = introspectionGraph.state.artifactsRoot or "";
    workspaceId = introspectionGraph.state.workspaceId or "";
  };

  diagnosticsHumanLines = [
    "INFO: diagnostics.local_overrides_active=${
      if diagnostics.localOverridesActive then "true" else "false"
    }"
    "INFO: diagnostics.local_override_count=${builtins.toString diagnostics.localOverrideCount}"
    "INFO: diagnostics.legacy_local_default_path=${diagnostics.legacyLocalDefault.path}"
    "INFO: diagnostics.legacy_local_default_present=${
      if diagnostics.legacyLocalDefault.present then "true" else "false"
    }"
    "INFO: diagnostics.legacy_local_default_customized=${
      if diagnostics.legacyLocalDefault.customized then "true" else "false"
    }"
    "INFO: diagnostics.legacy_local_default_active=${
      if diagnostics.legacyLocalDefault.active then "true" else "false"
    }"
    "INFO: diagnostics.legacy_local_default_status=${diagnostics.legacyLocalDefault.status}"
    "INFO: diagnostics.legacy_local_default_message=${diagnostics.legacyLocalDefault.message}"
    "INFO: diagnostics.workspace_marker_present=${
      if diagnostics.workspaceMarkerPresent then "true" else "false"
    }"
    "INFO: diagnostics.policy_id=${diagnostics.policyId}"
    "INFO: diagnostics.policy_kind=${diagnostics.policyKind}"
    "INFO: diagnostics.policy_source=${diagnostics.policySource}"
    "INFO: diagnostics.owner_scope=${diagnostics.ownerScope}"
    "INFO: diagnostics.discovery_scope=${diagnostics.discoveryScope}"
    "INFO: diagnostics.workspace_id=${diagnostics.workspaceId}"
    "INFO: diagnostics.runtime_base=${diagnostics.runtimeBase}"
    "INFO: diagnostics.registry_root=${diagnostics.registryRoot}"
    "INFO: diagnostics.artifacts_root=${diagnostics.artifactsRoot}"
  ];

  mkResolved =
    node:
    canonical.canonicalize {
      nodeId = node.nodeId;
      kind = node.kind;
      id = node.id;
      label = node.label;
    };

  mkResolution =
    node:
    canonical.canonicalize {
      ownerFiles = node.ownerFiles or [ ];
      summary = node.summary or "";
      description = node.description or "";
      data = node.data or { };
    };

  mkClosure =
    node:
    canonical.canonicalize {
      summary = node.closure or null;
      reasonChains = [ ];
    };

  mkTarget =
    node:
    canonical.canonicalize {
      nodeId = node.nodeId;
      kind = node.kind;
      id = node.id;
    };

  renderNodeCoreLines =
    node:
    let
      resolution = mkResolution node;
    in
    [
      "INFO: resolved.node=${node.nodeId}"
      "INFO: resolved.kind=${node.kind}"
      "INFO: resolved.id=${node.id}"
      "INFO: resolution.owner_files=${joinCsv resolution.ownerFiles}"
      "INFO: resolution.summary=${stringOrEmpty resolution.summary}"
    ];

  renderNodeExecutionLines =
    node:
    let
      execution = node.execution or null;
    in
    if execution == null then
      [ ]
    else
      [
        "INFO: execution.launcher_class=${execution.launcherClass or ""}"
        "INFO: execution.launcher_target=${stringOrEmpty (execution.launcherTarget or null)}"
        "INFO: execution.run_surface=${stringOrEmpty (execution.runSurface or null)}"
        "INFO: execution.mapped_tasks=${joinCsv (execution.mappedTaskIds or [ ])}"
        "INFO: execution.mapped_workflows=${joinCsv (execution.mappedWorkflowIds or [ ])}"
        "INFO: execution.mapped_service_sets=${joinCsv (execution.mappedServiceSetIds or [ ])}"
        "INFO: execution.selected_services=${joinCsv (execution.selectedServices or [ ])}"
      ];

  renderNodeClosureLines =
    node:
    let
      closure = node.closure or null;
      directPackages = if closure == null then [ ] else closure.directPackageNames or [ ];
      selectedServices = if closure == null then [ ] else closure.selectedServices or [ ];
    in
    if closure == null then
      [ ]
    else
      [
        "INFO: closure.direct_packages=${joinCsv directPackages}"
        "INFO: closure.selected_services=${joinCsv selectedServices}"
      ];

  graphEdges = introspectionGraph.edges or [ ];
  adjacency = builtins.listToAttrs (
    map (nodeId: {
      name = nodeId;
      value = builtins.filter (edge: edge.from == nodeId) graphEdges;
    }) nodeIds
  );

  chainFromPath =
    startNodeId: pathEdges:
    let
      visitedNodeIds = [ startNodeId ] ++ map (edge: edge.to) pathEdges;
      chainNodes = map (
        nodeId:
        let
          node = introspectionGraph.nodes.${nodeId};
        in
        canonical.canonicalize {
          nodeId = node.nodeId;
          kind = node.kind;
          id = node.id;
          label = node.label;
        }
      ) visitedNodeIds;
      chainEdges = map (
        edge:
        canonical.canonicalize {
          from = edge.from;
          to = edge.to;
          kind = edge.kind;
          reason = edge.reason;
          via = edge.via or null;
        }
      ) pathEdges;
    in
    canonical.canonicalize {
      length = builtins.length pathEdges;
      nodes = chainNodes;
      edges = chainEdges;
      rendered = builtins.concatStringsSep " -> " (map (node: node.nodeId) chainNodes);
    };

  sortChains =
    chains:
    builtins.sort (
      left: right:
      if left.length != right.length then left.length < right.length else left.rendered < right.rendered
    ) chains;

  findAllChains =
    startNodeId: targetNodeId:
    let
      walk =
        nodeId: visitedNodeIds: depth:
        if depth > maxWhyDepth then
          [ ]
        else if nodeId == targetNodeId then
          [ [ ] ]
        else
          builtins.concatLists (
            map (
              edge:
              if builtins.elem edge.to visitedNodeIds then
                [ ]
              else
                map (rest: [ edge ] ++ rest) (walk edge.to (visitedNodeIds ++ [ edge.to ]) (depth + 1))
            ) (adjacency.${nodeId} or [ ])
          );
    in
    if startNodeId == targetNodeId then
      [ ]
    else
      sortChains (map (path: chainFromPath startNodeId path) (walk startNodeId [ startNodeId ] 0));

  renderReverseLines =
    targetNodeId: chains:
    let
      bodyLines =
        if chains == [ ] then
          [ "WARN: no references found for ${targetNodeId}" ]
        else
          mapIndexed (
            index: chain: "INFO: reverse[${builtins.toString (index + 1)}]=${chain.rendered}"
          ) chains;
    in
    [ "INFO: reverse.target=${targetNodeId}" ] ++ bodyLines;

  renderWhyLines =
    sourceNodeId: targetNodeId: chains:
    if chains == [ ] then
      [ "WARN: no reason chains from ${sourceNodeId} to ${targetNodeId}" ]
    else
      mapIndexed (index: chain: "INFO: why[${builtins.toString (index + 1)}]=${chain.rendered}") chains;

  nodeViewFor =
    nodeId:
    let
      node = introspectionGraph.nodes.${nodeId};
      resolved = mkResolved node;
      resolution = mkResolution node;
      closure = mkClosure node;
      coreLines = renderNodeCoreLines node;
      executionLines = renderNodeExecutionLines node;
      closureLines = renderNodeClosureLines node;
    in
    canonical.canonicalize {
      jsonByMode = {
        default = canonical.canonicalize {
          query = null;
          inherit
            resolved
            resolution
            diagnostics
            ;
          execution = node.execution or null;
          inherit closure;
          reverse = null;
        };
        resolution = canonical.canonicalize {
          query = null;
          inherit
            resolved
            resolution
            diagnostics
            ;
        };
        execution = canonical.canonicalize {
          query = null;
          inherit
            resolved
            diagnostics
            ;
          execution = node.execution or null;
        };
        closure = canonical.canonicalize {
          query = null;
          inherit
            resolved
            diagnostics
            closure
            ;
          reverse = null;
        };
      };
      humanByMode = {
        default = coreLines ++ executionLines ++ closureLines;
        resolution = coreLines;
        execution = coreLines ++ executionLines;
        closure = coreLines ++ closureLines;
      };
    };

  reverseViewFor =
    targetNodeId:
    let
      targetNode = introspectionGraph.nodes.${targetNodeId};
      target = mkTarget targetNode;
      reasonChains = sortChains (
        builtins.concatLists (
          map (
            sourceNodeId: if sourceNodeId == targetNodeId then [ ] else findAllChains sourceNodeId targetNodeId
          ) nonExecutionNodeIds
        )
      );
      reasonChainsBySource = builtins.listToAttrs (
        builtins.filter (entry: entry.value != [ ]) (
          map (
            sourceNodeId:
            let
              chains = builtins.filter (chain: (builtins.head chain.nodes).nodeId == sourceNodeId) reasonChains;
            in
            {
              name = sourceNodeId;
              value = chains;
            }
          ) nonExecutionNodeIds
        )
      );
      whyLinesBySource = builtins.listToAttrs (
        builtins.filter (entry: entry.value != [ ]) (
          map (
            sourceNodeId:
            let
              chains = reasonChainsBySource.${sourceNodeId} or [ ];
            in
            {
              name = sourceNodeId;
              value = renderWhyLines sourceNodeId targetNodeId chains;
            }
          ) nonExecutionNodeIds
        )
      );
      reverseLines = renderReverseLines targetNodeId reasonChains;
      reverseResponse = canonical.canonicalize {
        query = null;
        resolved = null;
        resolution = null;
        execution = null;
        closure = null;
        reverse = {
          inherit
            target
            reasonChains
            ;
        };
        inherit diagnostics;
      };
    in
    canonical.canonicalize {
      inherit
        target
        reasonChains
        reasonChainsBySource
        whyLinesBySource
        ;
      jsonByMode = {
        default = reverseResponse;
        closure = reverseResponse;
        resolution = canonical.canonicalize {
          query = null;
          resolved = null;
          resolution = null;
          inherit diagnostics;
        };
        execution = canonical.canonicalize {
          query = null;
          resolved = null;
          execution = null;
          inherit diagnostics;
        };
      };
      humanByMode = {
        default = reverseLines;
        closure = reverseLines;
        resolution = [ ];
        execution = [ ];
      };
    };

  tokenEntriesForNode =
    nodeId:
    let
      node = introspectionGraph.nodes.${nodeId};
      data = node.data or { };
      tokenValues =
        if node.kind == "service" then
          uniqueSorted (nonEmptyStrings [
            node.id
            (data.serviceId or "")
          ])
        else if node.kind == "service-set" then
          uniqueSorted (nonEmptyStrings [
            node.id
            (data.serviceSetId or "")
          ])
        else
          uniqueSorted (nonEmptyStrings [ node.id ]);
      explicitEntries = map (value: {
        token = "${node.kind}:${value}";
        kind = node.kind;
        entry = canonical.canonicalize {
          nodeId = node.nodeId;
          sourceKind = "explicit";
          selector = node.kind;
        };
      }) tokenValues;
      bareEntries = map (value: {
        token = value;
        kind = node.kind;
        entry = canonical.canonicalize {
          nodeId = node.nodeId;
          sourceKind = "bare";
          selector = node.kind;
        };
      }) tokenValues;
    in
    {
      inherit
        explicitEntries
        bareEntries
        ;
    };

  allTokenEntries = builtins.listToAttrs (
    map (nodeId: {
      name = nodeId;
      value = tokenEntriesForNode nodeId;
    }) nonExecutionNodeIds
  );

  explicitEntries = builtins.concatLists (
    map (nodeId: allTokenEntries.${nodeId}.explicitEntries) nonExecutionNodeIds
  );
  bareEntries = builtins.concatLists (
    map (nodeId: allTokenEntries.${nodeId}.bareEntries) nonExecutionNodeIds
  );

  groupEntriesByToken =
    entries:
    builtins.foldl' (
      acc: entry:
      acc
      // {
        ${entry.token} = (acc.${entry.token} or [ ]) ++ [ entry.entry ];
      }
    ) { } entries;

  buildExplicitLookup =
    entries:
    let
      grouped = groupEntriesByToken entries;
      tokenNames = sortNames grouped;
      duplicateTokens = builtins.filter (token: builtins.length grouped.${token} > 1) tokenNames;
    in
    if duplicateTokens != [ ] then
      throw "compile-introspection-bundle: duplicate explicit selectors: ${builtins.concatStringsSep ", " duplicateTokens}"
    else
      builtins.listToAttrs (
        map (token: {
          name = token;
          value = builtins.head grouped.${token};
        }) tokenNames
      );

  buildBareLookup =
    entries:
    let
      grouped = groupEntriesByToken entries;
      tokenNames = sortNames grouped;
      ambiguousTokens = builtins.filter (token: builtins.length grouped.${token} > 1) tokenNames;
      resolvedTokens = builtins.filter (token: builtins.length grouped.${token} == 1) tokenNames;
    in
    {
      resolved = builtins.listToAttrs (
        map (token: {
          name = token;
          value = builtins.head grouped.${token};
        }) resolvedTokens
      );
      ambiguous = builtins.listToAttrs (
        map (token: {
          name = token;
          value = uniqueSorted (map (entry: entry.nodeId) grouped.${token});
        }) ambiguousTokens
      );
    };

  bareLookupAny = buildBareLookup bareEntries;
  bareLookupByKind = builtins.listToAttrs (
    map (
      kind:
      let
        lookup = buildBareLookup (builtins.filter (entry: entry.kind == kind) bareEntries);
      in
      {
        name = kind;
        value = lookup;
      }
    ) kindNames
  );

  nodeViews = builtins.listToAttrs (
    map (nodeId: {
      name = nodeId;
      value = nodeViewFor nodeId;
    }) nonExecutionNodeIds
  );

  reverseViews = builtins.listToAttrs (
    map (nodeId: {
      name = nodeId;
      value = reverseViewFor nodeId;
    }) nonExecutionNodeIds
  );
in
canonical.canonicalize {
  schema = {
    kind = "nixfied-introspection-bundle";
    version = 1;
  };
  validKinds = kindNames;
  diagnosticsHumanLines = diagnosticsHumanLines;
  resolutionIndex = {
    explicit = buildExplicitLookup explicitEntries;
    bare = {
      any = bareLookupAny.resolved;
      byKind = builtins.listToAttrs (
        map (kind: {
          name = kind;
          value = bareLookupByKind.${kind}.resolved;
        }) kindNames
      );
    };
    ambiguousBare = {
      any = bareLookupAny.ambiguous;
      byKind = builtins.listToAttrs (
        map (kind: {
          name = kind;
          value = bareLookupByKind.${kind}.ambiguous;
        }) kindNames
      );
    };
  };
  nodeViews = nodeViews;
  reverseViews = reverseViews;
}
