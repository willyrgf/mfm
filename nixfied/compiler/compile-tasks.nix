{
  lib,
  canonical,
  idLib,
}:
{
  resolved,
  runtime,
}:
let
  rawTasks = resolved.tasks or { };
  names = builtins.sort builtins.lessThan (builtins.attrNames rawTasks);
  globalRuntimeInputs = map builtins.toString (resolved.tooling.runtimePackages or [ ]);

  unique =
    list:
    builtins.foldl' (acc: value: if builtins.elem value acc then acc else acc ++ [ value ]) [ ] list;

  normalizeHooks =
    hooks:
    builtins.mapAttrs (_: hook: {
      command = hook.command;
      runtimeInputs = map builtins.toString hook.runtimeInputs;
      passThroughEnv = hook.passThroughEnv;
      env = hook.env;
      workdir = hook.workdir;
      customWorkdir = hook.customWorkdir;
    }) hooks;

  normalizeId =
    name: rawId:
    let
      effective = if rawId == "" || rawId == name then "task.${idLib.sanitize name}" else rawId;
    in
    idLib.ensurePrefix "task" effective;

  normalizeTask =
    name:
    let
      raw = rawTasks.${name};
      id = normalizeId name raw.id;
    in
    canonical.canonicalize {
      inherit
        id
        ;

      kind = raw.kind;
      summary = raw.summary;
      description = raw.description;
      tags = raw.tags;

      runner = {
        type = raw.runner.type;
        command = raw.runner.command;
        package = if raw.runner.package == null then null else builtins.toString raw.runner.package;
        workflowId = raw.runner.workflowId;
      };

      contract = raw.contract;

      runtime = {
        slotEnv = raw.runtime.slotEnv;
        workdir = raw.runtime.workdir;
        customWorkdir = raw.runtime.customWorkdir;
        hermetic = raw.runtime.hermetic;
        runtimeInputs = unique (map builtins.toString raw.runtime.runtimeInputs ++ globalRuntimeInputs);
        passThroughEnv = raw.runtime.passThroughEnv;
        logging = raw.runtime.logging;
        env = raw.runtime.env;
        umask = raw.runtime.umask;
        locale = raw.runtime.locale;
        timezone = raw.runtime.timezone;
        preHooks = normalizeHooks raw.runtime.preHooks;
        postHooks = normalizeHooks raw.runtime.postHooks;
      };

      scheduling = raw.scheduling;
      deps = raw.deps;
      produces = raw.produces;
      ui = raw.ui;
    };

  addTask =
    acc: name:
    let
      task = normalizeTask name;
      id = task.id;
    in
    if builtins.hasAttr id acc then
      if canonical.toCanonicalNix acc.${id} == canonical.toCanonicalNix task then
        acc
      else
        throw "task id collision for '${id}'"
    else
      acc
      // {
        ${id} = task;
      };

  tasksById = builtins.foldl' addTask { } names;
  ids = builtins.sort builtins.lessThan (builtins.attrNames tasksById);
in
builtins.listToAttrs (
  map (id: {
    name = id;
    value = tasksById.${id};
  }) ids
)
