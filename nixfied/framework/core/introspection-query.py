#!/usr/bin/env python3

import json
import sys


VALID_KINDS = {"app", "task", "workflow", "service", "service-set", "package"}
SELECTOR_PREFIXES = VALID_KINDS


def fail(message):
    print(f"ERROR: {message}", file=sys.stderr)
    sys.exit(2)


def print_help():
    print(
        """Usage: nix run .#introspect -- [query] [--json] [--kind <kind>] [--why <target>] [--reverse <target>] [--resolution-only|--execution|--closure]

Examples:
  nix run .#introspect -- check
  nix run .#introspect -- app:check --json
  nix run .#introspect -- task:task.check
  nix run .#introspect -- workflow:workflow.ci.full
  nix run .#introspect -- service:postgres
  nix run .#introspect -- service-set:default
  nix run .#introspect -- check --why package:nix-checks
  nix run .#introspect -- reverse package:nix-checks
"""
    )


def parse_args(argv):
    opts = {
        "json": False,
        "kind": None,
        "why": None,
        "reverse": None,
        "resolution_only": False,
        "execution_only": False,
        "closure_only": False,
    }
    positionals = []
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg in ("-h", "--help"):
            print_help()
            sys.exit(0)
        if arg == "--json":
            opts["json"] = True
            i += 1
            continue
        if arg == "--kind":
            i += 1
            if i >= len(argv):
                fail("--kind requires a value")
            opts["kind"] = argv[i]
            i += 1
            continue
        if arg == "--why":
            i += 1
            if i >= len(argv):
                fail("--why requires a value")
            opts["why"] = argv[i]
            i += 1
            continue
        if arg == "--reverse":
            i += 1
            if i >= len(argv):
                fail("--reverse requires a value")
            opts["reverse"] = argv[i]
            i += 1
            continue
        if arg == "--resolution-only":
            opts["resolution_only"] = True
            i += 1
            continue
        if arg == "--execution":
            opts["execution_only"] = True
            i += 1
            continue
        if arg == "--closure":
            opts["closure_only"] = True
            i += 1
            continue

        positionals.append(arg)
        i += 1

    if opts["kind"] is not None and opts["kind"] not in VALID_KINDS:
        fail(f"--kind must be one of: {', '.join(sorted(VALID_KINDS))}")

    view_flags = [
        opts["resolution_only"],
        opts["execution_only"],
        opts["closure_only"],
    ]
    if sum(1 for flag in view_flags if flag) > 1:
        fail("only one of --resolution-only, --execution, or --closure may be used")

    if opts["why"] and opts["reverse"]:
        fail("--why and --reverse cannot be used together")

    if positionals and positionals[0] == "reverse":
        if opts["reverse"] is not None:
            fail("reverse target was provided twice")
        if len(positionals) != 2:
            fail("reverse mode requires exactly one target")
        opts["reverse"] = positionals[1]
        positionals = []

    if len(positionals) > 1:
        fail("expected at most one query token")

    query = positionals[0] if positionals else None

    if opts["why"] and query is None:
        fail("--why requires a query token")

    if opts["reverse"] and query is not None:
        fail("reverse mode does not accept a separate query token")

    if query is None and opts["reverse"] is None:
        fail("expected a query token or reverse target")

    return query, opts


def load_graph(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def explicit_selector(token):
    prefix, sep, value = token.partition(":")
    if sep and prefix in SELECTOR_PREFIXES:
        return prefix, value
    return None, None


def resolve_explicit(graph, kind, value):
    nodes = graph["nodes"]
    if kind == "app":
        node_id = f"app:{value}"
        return nodes.get(node_id)
    if kind == "task":
        node_id = f"task:{value}"
        return nodes.get(node_id)
    if kind == "workflow":
        node_id = f"workflow:{value}"
        return nodes.get(node_id)
    if kind == "service":
        node = nodes.get(f"service:{value}")
        if node is not None:
            return node
        for candidate in nodes.values():
            if candidate["kind"] != "service":
                continue
            if candidate.get("data", {}).get("serviceId") == value:
                return candidate
        return None
    if kind == "service-set":
        node = nodes.get(f"service-set:{value}")
        if node is not None:
            return node
        for candidate in nodes.values():
            if candidate["kind"] != "service-set":
                continue
            if candidate.get("data", {}).get("serviceSetId") == value:
                return candidate
        return None
    if kind == "package":
        return nodes.get(f"package:{value}")
    return None


def resolve_bare(graph, token, kind_filter):
    candidates = []
    for node in graph["nodes"].values():
        if node["kind"] == "execution":
            continue
        if kind_filter and node["kind"] != kind_filter:
            continue
        if node["kind"] == "app" and node["id"] == token:
            candidates.append(node)
            continue
        if node["kind"] == "task" and node["id"] == token:
            candidates.append(node)
            continue
        if node["kind"] == "workflow" and node["id"] == token:
            candidates.append(node)
            continue
        if node["kind"] == "service":
            if node["id"] == token or node.get("data", {}).get("serviceId") == token:
                candidates.append(node)
                continue
        if node["kind"] == "service-set":
            if node["id"] == token or node.get("data", {}).get("serviceSetId") == token:
                candidates.append(node)
                continue
        if node["kind"] == "package" and node["id"] == token:
            candidates.append(node)
            continue
    return sorted(candidates, key=lambda node: node["nodeId"])


def resolve_query(graph, token, kind_filter):
    prefix, value = explicit_selector(token)
    if prefix is not None:
        if kind_filter and prefix != kind_filter:
            fail(f"query '{token}' does not match --kind {kind_filter}")
        node = resolve_explicit(graph, prefix, value)
        if node is None:
            fail(f"unable to resolve '{token}'")
        return node, {"source_kind": "explicit", "selector": prefix}

    candidates = resolve_bare(graph, token, kind_filter)
    if not candidates:
        fail(f"unable to resolve '{token}'")
    if len(candidates) > 1:
        labels = ", ".join(node["nodeId"] for node in candidates)
        fail(f"ambiguous query '{token}' matched: {labels}")
    return candidates[0], {"source_kind": "bare", "selector": candidates[0]["kind"]}


def build_adjacency(edges):
    forward = {}
    reverse = {}
    for edge in edges:
        forward.setdefault(edge["from"], []).append(edge)
        reverse.setdefault(edge["to"], []).append(edge)
    for mapping in (forward, reverse):
        for key in mapping:
            mapping[key] = sorted(
                mapping[key],
                key=lambda edge: (
                    edge["to"] if mapping is forward else edge["from"],
                    edge["kind"],
                    edge["reason"],
                ),
            )
    return forward, reverse


def find_all_paths(start_node_id, end_node_id, adjacency, max_depth=12):
    paths = []

    def walk(node_id, visited_nodes, path_edges):
        if len(path_edges) > max_depth:
            return
        if node_id == end_node_id:
            paths.append(list(path_edges))
            return
        for edge in adjacency.get(node_id, []):
            next_node_id = edge["to"]
            if next_node_id in visited_nodes:
                continue
            visited_nodes.add(next_node_id)
            path_edges.append(edge)
            walk(next_node_id, visited_nodes, path_edges)
            path_edges.pop()
            visited_nodes.remove(next_node_id)

    walk(start_node_id, {start_node_id}, [])
    return sorted(paths, key=lambda path: (len(path), [edge["to"] for edge in path], [edge["reason"] for edge in path]))


def path_to_chain(graph, start_node_id, path_edges):
    nodes = [graph["nodes"][start_node_id]]
    edges = []
    current = start_node_id
    for edge in path_edges:
        if edge["from"] != current:
            fail("introspection graph contains an invalid path edge ordering")
        edges.append(edge)
        current = edge["to"]
        nodes.append(graph["nodes"][current])
    return {
        "length": len(path_edges),
        "nodes": [
            {
                "nodeId": node["nodeId"],
                "kind": node["kind"],
                "id": node["id"],
                "label": node["label"],
            }
            for node in nodes
        ],
        "edges": [
            {
                "from": edge["from"],
                "to": edge["to"],
                "kind": edge["kind"],
                "reason": edge["reason"],
                "via": edge.get("via"),
            }
            for edge in edges
        ],
        "rendered": " -> ".join(node["nodeId"] for node in nodes),
    }


def collect_reverse_chains(graph, target_node_id, adjacency):
    chains = []
    for node in graph["nodes"].values():
        if node["kind"] == "execution":
            continue
        if node["nodeId"] == target_node_id:
            continue
        for path in find_all_paths(node["nodeId"], target_node_id, adjacency):
            chains.append(path_to_chain(graph, node["nodeId"], path))
    chains.sort(key=lambda chain: (chain["length"], chain["rendered"]))
    return chains


def diagnostics(graph):
    return {
        "localOverridesActive": graph["localOverrides"]["active"],
        "localOverrideCount": graph["localOverrides"]["count"],
        "legacyLocalDefault": graph["legacyLocalDefault"],
        "policyId": graph["state"]["policyId"],
        "policyKind": graph["state"]["policyKind"],
        "policySource": graph["state"]["policySource"],
        "ownerScope": graph["state"]["ownerScope"],
        "discoveryScope": graph["state"]["discoveryScope"],
        "workspaceMarkerPresent": graph["state"]["workspaceMarkerPresent"],
        "runtimeBase": graph["state"]["runtimeBase"],
        "registryRoot": graph["state"]["registryRoot"],
        "artifactsRoot": graph["state"]["artifactsRoot"],
        "workspaceId": graph["state"]["workspaceId"],
    }


def response_for_query(graph, node, query_meta, why_target, adjacency):
    response = {
        "query": {
            "raw": query_meta["raw"],
            "sourceKind": query_meta["source_kind"],
            "selector": query_meta["selector"],
        },
        "resolved": {
            "nodeId": node["nodeId"],
            "kind": node["kind"],
            "id": node["id"],
            "label": node["label"],
        },
        "resolution": {
            "ownerFiles": node.get("ownerFiles", []),
            "summary": node.get("summary", ""),
            "description": node.get("description", ""),
            "data": node.get("data", {}),
        },
        "execution": node.get("execution"),
        "closure": {
            "summary": node.get("closure"),
            "reasonChains": [],
        },
        "reverse": None,
        "diagnostics": diagnostics(graph),
    }

    if why_target is not None:
        target_node, target_meta = resolve_query(graph, why_target, None)
        chains = [
            path_to_chain(graph, node["nodeId"], path)
            for path in find_all_paths(node["nodeId"], target_node["nodeId"], adjacency)
        ]
        response["closure"] = {
            "target": {
                "raw": why_target,
                "sourceKind": target_meta["source_kind"],
                "selector": target_meta["selector"],
                "nodeId": target_node["nodeId"],
                "kind": target_node["kind"],
                "id": target_node["id"],
            },
            "summary": node.get("closure"),
            "reasonChains": chains,
        }

    return response


def response_for_reverse(graph, target_token, adjacency):
    target_node, target_meta = resolve_query(graph, target_token, None)
    return {
        "query": None,
        "resolved": None,
        "resolution": None,
        "execution": None,
        "closure": None,
        "reverse": {
            "target": {
                "raw": target_token,
                "sourceKind": target_meta["source_kind"],
                "selector": target_meta["selector"],
                "nodeId": target_node["nodeId"],
                "kind": target_node["kind"],
                "id": target_node["id"],
            },
            "reasonChains": collect_reverse_chains(graph, target_node["nodeId"], adjacency),
        },
        "diagnostics": diagnostics(graph),
    }


def trim_response(response, opts):
    if opts["resolution_only"]:
        return {
            "query": response["query"],
            "resolved": response["resolved"],
            "resolution": response["resolution"],
            "diagnostics": response["diagnostics"],
        }
    if opts["execution_only"]:
        return {
            "query": response["query"],
            "resolved": response["resolved"],
            "execution": response["execution"],
            "diagnostics": response["diagnostics"],
        }
    if opts["closure_only"]:
        return {
            "query": response["query"],
            "resolved": response["resolved"],
            "closure": response["closure"],
            "reverse": response["reverse"],
            "diagnostics": response["diagnostics"],
        }
    return response


def print_human(response):
    if response.get("query") is not None:
        print(f"INFO: query.raw={response['query']['raw']}")
        print(f"INFO: query.source_kind={response['query']['sourceKind']}")
        print(f"INFO: query.selector={response['query']['selector']}")
        print(f"INFO: resolved.node={response['resolved']['nodeId']}")
        print(f"INFO: resolved.kind={response['resolved']['kind']}")
        print(f"INFO: resolved.id={response['resolved']['id']}")
        owner_files = ",".join(response["resolution"]["ownerFiles"])
        print(f"INFO: resolution.owner_files={owner_files}")
        print(f"INFO: resolution.summary={response['resolution']['summary']}")
        if response["execution"] is not None:
            execution = response["execution"]
            tasks = ",".join(execution.get("mappedTaskIds", []))
            workflows = ",".join(execution.get("mappedWorkflowIds", []))
            service_sets = ",".join(execution.get("mappedServiceSetIds", []))
            services = ",".join(execution.get("selectedServices", []))
            print(f"INFO: execution.launcher_class={execution.get('launcherClass')}")
            print(f"INFO: execution.launcher_target={execution.get('launcherTarget')}")
            print(f"INFO: execution.run_surface={execution.get('runSurface')}")
            print(f"INFO: execution.mapped_tasks={tasks}")
            print(f"INFO: execution.mapped_workflows={workflows}")
            print(f"INFO: execution.mapped_service_sets={service_sets}")
            print(f"INFO: execution.selected_services={services}")
        if response.get("closure") is not None and response["closure"] is not None:
            closure = response["closure"]
            if closure.get("summary") is not None:
                summary = closure["summary"]
                direct_packages = ",".join((summary or {}).get("directPackageNames", []))
                selected_services = ",".join((summary or {}).get("selectedServices", []))
                print(f"INFO: closure.direct_packages={direct_packages}")
                print(f"INFO: closure.selected_services={selected_services}")
            for index, chain in enumerate(closure.get("reasonChains", []), start=1):
                print(f"INFO: why[{index}]={chain['rendered']}")
            if closure.get("target") and not closure.get("reasonChains"):
                print(
                    f"WARN: no reason chains from {response['resolved']['nodeId']} to {closure['target']['nodeId']}"
                )

    if response.get("reverse") is not None and response["reverse"] is not None:
        reverse = response["reverse"]
        print(f"INFO: reverse.target={reverse['target']['nodeId']}")
        for index, chain in enumerate(reverse.get("reasonChains", []), start=1):
            print(f"INFO: reverse[{index}]={chain['rendered']}")
        if not reverse.get("reasonChains"):
            print(f"WARN: no references found for {reverse['target']['nodeId']}")

    diag = response["diagnostics"]
    print(f"INFO: diagnostics.local_overrides_active={str(diag['localOverridesActive']).lower()}")
    print(f"INFO: diagnostics.local_override_count={diag['localOverrideCount']}")
    legacy_local_default = diag["legacyLocalDefault"]
    print(f"INFO: diagnostics.legacy_local_default_path={legacy_local_default['path']}")
    print(f"INFO: diagnostics.legacy_local_default_present={str(legacy_local_default['present']).lower()}")
    print(f"INFO: diagnostics.legacy_local_default_customized={str(legacy_local_default['customized']).lower()}")
    print(f"INFO: diagnostics.legacy_local_default_active={str(legacy_local_default['active']).lower()}")
    print(f"INFO: diagnostics.legacy_local_default_status={legacy_local_default['status']}")
    print(f"INFO: diagnostics.legacy_local_default_message={legacy_local_default['message']}")
    print(f"INFO: diagnostics.workspace_marker_present={str(diag['workspaceMarkerPresent']).lower()}")
    print(f"INFO: diagnostics.policy_id={diag['policyId']}")
    print(f"INFO: diagnostics.policy_kind={diag['policyKind']}")
    print(f"INFO: diagnostics.policy_source={diag['policySource']}")
    print(f"INFO: diagnostics.owner_scope={diag['ownerScope']}")
    print(f"INFO: diagnostics.discovery_scope={diag['discoveryScope']}")
    print(f"INFO: diagnostics.workspace_id={diag['workspaceId']}")
    print(f"INFO: diagnostics.runtime_base={diag['runtimeBase']}")
    print(f"INFO: diagnostics.registry_root={diag['registryRoot']}")
    print(f"INFO: diagnostics.artifacts_root={diag['artifactsRoot']}")


def main(argv):
    if len(argv) < 2:
        fail("expected path to introspection graph")

    graph_path = argv[1]
    query, opts = parse_args(argv[2:])
    graph = load_graph(graph_path)
    adjacency, _ = build_adjacency(graph["edges"])

    if opts["reverse"] is not None:
        response = response_for_reverse(graph, opts["reverse"], adjacency)
    else:
        node, query_meta = resolve_query(graph, query, opts["kind"])
        query_meta["raw"] = query
        response = response_for_query(graph, node, query_meta, opts["why"], adjacency)

    response = trim_response(response, opts)

    if opts["json"]:
        print(json.dumps(response, indent=2, sort_keys=True))
    else:
        print_human(response)


if __name__ == "__main__":
    main(sys.argv)
