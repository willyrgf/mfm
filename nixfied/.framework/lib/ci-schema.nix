# CI configuration schema validation and normalization.
{
  pkgs,
  knownApps ? [ ],
}:

let
  lib = pkgs.lib;
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    expect
    renderErrors
    isNonEmptyString
    isNonEmptyList
    isListOfNonEmptyStrings
    sortedAttrNames
    isEnvVarName
    isScalarAttrset
    ;
  actionSchema = import ./action-schema.nix {
    inherit pkgs knownApps;
  };

  renderWhen =
    when:
    let
      envEquals = when.envEquals or { };
      envPresent = when.envPresent or [ ];
      envEqualsTests = map (
        key: "[ \"\${" + key + ":-}\" = " + lib.escapeShellArg (toString envEquals.${key}) + " ]"
      ) (sortedAttrNames envEquals);
      envPresentTests = map (key: "[ -n \"\${" + key + ":-}\" ]") envPresent;
      tests = envEqualsTests ++ envPresentTests;
    in
    if tests == [ ] then "" else lib.concatStringsSep " && " tests;

  validateWhenErrors =
    {
      stepName,
      when,
    }:
    let
      prefix = "ci.steps.${stepName}.when";
      envEquals = when.envEquals or { };
      envPresent = when.envPresent or [ ];
    in
    if !builtins.isAttrs when then
      [ "${prefix}: must be an attribute set" ]
    else
      expect (
        !(when ? envEquals) || isScalarAttrset envEquals
      ) "${prefix}.envEquals must be an attrset of shell-safe env keys to scalar values"
      ++ expect (
        !(when ? envPresent) || (builtins.isList envPresent && builtins.all isEnvVarName envPresent)
      ) "${prefix}.envPresent must be a list of shell-safe env var tokens";

  validateStepErrors =
    {
      stepName,
      step,
      knownStepNames,
    }:
    let
      prefix = "ci.steps.${stepName}";
      depends = step.dependsOn or [ ];
      unknownDepends = builtins.filter (dep: !(builtins.elem dep knownStepNames)) depends;
      whenErrs =
        if step ? when then
          validateWhenErrors {
            inherit stepName;
            when = step.when;
          }
        else
          [ ];
      stepActions = step.actions or [ ];
      cleanupActions = step.cleanupActions or [ ];
    in
    if !builtins.isAttrs step then
      [ "${prefix}: step config must be an attribute set" ]
    else
      expect (!(step ? run)) "${prefix}.run has been removed. Use ${prefix}.actions."
      ++ expect (!(step ? cleanup)) "${prefix}.cleanup has been removed. Use ${prefix}.cleanupActions."
      ++ expect (
        !(step ? when) || builtins.isAttrs step.when
      ) "${prefix}.when must be an attribute set (string conditions are not supported)"
      ++ expect (
        !(step ? description) || builtins.isString step.description
      ) "${prefix}.description must be a string when set"
      ++ expect (
        !(step ? skipIfMissing)
        || (builtins.isList (step.skipIfMissing) && isListOfNonEmptyStrings (step.skipIfMissing))
      ) "${prefix}.skipIfMissing must be a list of non-empty strings"
      ++ expect (
        builtins.isList depends && isListOfNonEmptyStrings depends
      ) "${prefix}.dependsOn must be a list of non-empty strings"
      ++ expect (
        unknownDepends == [ ]
      ) "${prefix}.dependsOn references unknown steps: ${builtins.concatStringsSep ", " unknownDepends}"
      ++ expect (
        !(step ? env) || isScalarAttrset (step.env or { })
      ) "${prefix}.env must be an attrset with shell-safe keys and scalar values"
      ++ whenErrs
      ++ actionSchema.validateActionListErrors {
        context = "${prefix}.actions";
        actions = stepActions;
        required = true;
      }
      ++ actionSchema.validateActionListErrors {
        context = "${prefix}.cleanupActions";
        actions = cleanupActions;
        required = false;
      };

  validateModeErrors =
    {
      modeName,
      modeCfg,
      knownStepNames,
    }:
    let
      prefix = "ci.modes.${modeName}";
      modeSteps = modeCfg.steps or [ ];
      unknown = builtins.filter (stepName: !(builtins.elem stepName knownStepNames)) modeSteps;
    in
    if !builtins.isAttrs modeCfg then
      [ "${prefix}: mode config must be an attribute set" ]
    else
      expect (isNonEmptyList modeSteps) "${prefix}.steps must be a non-empty list"
      ++ expect (isListOfNonEmptyStrings modeSteps) "${prefix}.steps must be a list of non-empty strings"
      ++ expect (
        unknown == [ ]
      ) "${prefix}.steps references unknown steps: ${builtins.concatStringsSep ", " unknown}";

  hasCycleFrom =
    steps: stepNames: path: name:
    let
      deps = if builtins.hasAttr name steps then (steps.${name}.dependsOn or [ ]) else [ ];
    in
    builtins.any (
      dep:
      if builtins.elem dep path then
        true
      else if !(builtins.elem dep stepNames) then
        false
      else
        hasCycleFrom steps stepNames (path ++ [ name ]) dep
    ) deps;

  validateCi =
    ci:
    let
      stepsRaw = ci.steps or { };
      modes = ci.modes or { };
      stepNames = sortedAttrNames stepsRaw;
      modeNames = sortedAttrNames modes;

      baseErrs =
        expect (!(ci ? setup)) "ci.setup has been removed. Use ci.setupActions."
        ++ expect (!(ci ? teardown)) "ci.teardown has been removed. Use ci.teardownActions."
        ++ expect (!(ci ? stepCommand)) "ci.stepCommand has been removed. Define per-step actions."
        ++ expect (!(ci ? modeCommand)) "ci.modeCommand has been removed. Define ci.modes and ci.steps."
        ++ expect (builtins.isAttrs stepsRaw) "ci.steps must be an attribute set"
        ++ expect (builtins.isAttrs modes) "ci.modes must be an attribute set"
        ++ expect (modeNames != [ ]) "ci.modes must define at least one mode";

      stepErrs = builtins.concatLists (
        map (
          name:
          validateStepErrors {
            stepName = name;
            step = stepsRaw.${name};
            knownStepNames = stepNames;
          }
        ) stepNames
      );

      modeErrs = builtins.concatLists (
        map (
          mode:
          validateModeErrors {
            modeName = mode;
            modeCfg = modes.${mode};
            knownStepNames = stepNames;
          }
        ) modeNames
      );

      cyclicSteps = lib.unique (
        builtins.filter (name: hasCycleFrom stepsRaw stepNames [ ] name) stepNames
      );

      cycleErrs = expect (
        cyclicSteps == [ ]
      ) "ci.steps dependency graph has cycle(s): ${builtins.concatStringsSep ", " cyclicSteps}";

      setupActions = ci.setupActions or [ ];
      teardownActions = ci.teardownActions or [ ];
      rootActionErrs =
        actionSchema.validateActionListErrors {
          context = "ci.setupActions";
          actions = setupActions;
          required = false;
        }
        ++ actionSchema.validateActionListErrors {
          context = "ci.teardownActions";
          actions = teardownActions;
          required = false;
        };

      errs = baseErrs ++ stepErrs ++ modeErrs ++ cycleErrs ++ rootActionErrs;

      normalizedSteps = builtins.listToAttrs (
        map (
          name:
          let
            raw = stepsRaw.${name};
          in
          {
            inherit name;
            value = raw // {
              run = actionSchema.renderActionList (raw.actions or [ ]);
              cleanup = actionSchema.renderActionList (raw.cleanupActions or [ ]);
              when = renderWhen (raw.when or { });
              skip_if_missing = raw.skipIfMissing or [ ];
              depends_on = raw.dependsOn or [ ];
            };
          }
        ) stepNames
      );
    in
    if errs == [ ] then
      {
        steps = normalizedSteps;
        inherit modes;
        setupScript = actionSchema.renderActionList setupActions;
        teardownScript = actionSchema.renderActionList teardownActions;
      }
    else
      throw ''
        Nixfied CI config violated:
        ${renderErrors errs}

        Use typed action fields:
          - ci.setupActions / ci.teardownActions
          - ci.steps.<name>.actions / ci.steps.<name>.cleanupActions
          - ci.steps.<name>.when = { envEquals = { ...; }; envPresent = [ ... ]; }
      '';
in
{
  inherit validateCi;
}
