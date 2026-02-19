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
        runtimeInputs = map builtins.toString raw.runtime.runtimeInputs;
        passThroughEnv = raw.runtime.passThroughEnv;
        env = raw.runtime.env;
        umask = raw.runtime.umask;
        locale = raw.runtime.locale;
        timezone = raw.runtime.timezone;
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
