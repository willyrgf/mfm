{ lib, canonical }:
{
  system,
  projectRoot,
  resolved,
  runtime,
  services,
  tasks,
  workflows,
  features,
  views,
}:
let
  evalHash = canonical.hashCanonical {
    schema = {
      kind = "nixfied-model-eval";
      version = 1;
    };
    identity = resolved.identity;
    runtime = runtime;
    services = services;
    tasks = tasks;
    workflows = workflows;
    features = features;
  };

  model = canonical.canonicalize {
    schema = {
      kind = "nixfied-model";
      version = 1;
    };

    identity = {
      projectId = resolved.identity.projectId;
      projectName = resolved.identity.projectName;
      description = resolved.identity.description;
      workspaceId = resolved.state.workspaceId;
      system = system;
      evalHash = evalHash;
    };

    runtime = runtime;

    services = services;
    tasks = tasks;
    workflows = workflows;
    features = features;

    views = {
      apps = views.apps;
      help = views.help;
      docs = views.docs;
      features = views.features;
    };

    state = {
      workspaceId = resolved.state.workspaceId;
      registry = {
        schemaVersion = 1;
        root = resolved.state.registryRoot;
      };
      artifacts = {
        root = resolved.state.artifactsRoot;
      };
    };
  };

  stateHash = canonical.hashCanonical model;
in
{
  inherit
    model
    stateHash
    ;
}
