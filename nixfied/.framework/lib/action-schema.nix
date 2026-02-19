# Shared typed action schema for CI/isolation execution.
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
    isListOfNonEmptyStrings
    isScalar
    isEnvVarName
    isScalarAttrset
    ;

  actionKinds = [
    "runApp"
    "exec"
    "logCapture"
    "artifactTouch"
    "assertEnvEquals"
    "fail"
  ];

  isArtifactName =
    value: builtins.isString value && (builtins.match "^[A-Za-z0-9._-]+$" value) != null;

  renderEnvExports =
    env:
    let
      keys = lib.sort (a: b: a < b) (builtins.attrNames env);
    in
    lib.concatMapStringsSep "\n" (
      key: "export ${key}=${lib.escapeShellArg (toString env.${key})}"
    ) keys;

  renderArgv =
    argv:
    let
      head = lib.escapeShellArg (builtins.elemAt argv 0);
      tail = builtins.tail argv;
      tailArgs = lib.concatMapStringsSep " " lib.escapeShellArg tail;
    in
    if tail == [ ] then head else "${head} ${tailArgs}";

  renderAction =
    action:
    let
      kind = action.kind;
      envBlock = if action ? env && action.env != { } then renderEnvExports action.env else "";
      withEnv = body: ''
        (
          ${envBlock}
          ${body}
        )
      '';
    in
    if kind == "runApp" then
      let
        args = action.args or [ ];
        argsInit =
          if args == [ ] then
            ""
          else
            "_NIXFIED_RUN_APP_ARGS+=(${lib.concatMapStringsSep " " lib.escapeShellArg args})";
        appRef = lib.escapeShellArg ".#${action.app}";
      in
      withEnv ''
        _NIXFIED_RUN_APP_ARGS=()
        ${argsInit}
        if [ "${
          if action.passArgs or false then "1" else "0"
        }" = "1" ] && [ "''${CI_STEP_ARGS+x}" = "x" ] && [ "''${#CI_STEP_ARGS[@]}" -gt 0 ]; then
          _NIXFIED_RUN_APP_ARGS+=("''${CI_STEP_ARGS[@]}")
        fi
        if [ "''${#_NIXFIED_RUN_APP_ARGS[@]}" -gt 0 ]; then
          nix run ${appRef} -- "''${_NIXFIED_RUN_APP_ARGS[@]}"
        else
          nix run ${appRef}
        fi
      ''
    else if kind == "exec" then
      withEnv (renderArgv action.argv)
    else if kind == "logCapture" then
      withEnv ''
        _NIXFIED_LOGFILE="$(artifact_path ${lib.escapeShellArg action.artifact})"
        log_capture "$_NIXFIED_LOGFILE" -- ${renderArgv action.argv}
      ''
    else if kind == "artifactTouch" then
      ''touch "$(artifact_path ${lib.escapeShellArg action.artifact})"''
    else if kind == "assertEnvEquals" then
      let
        varRef = "\${" + action.name + ":-}";
      in
      ''
        if [ "${varRef}" != ${lib.escapeShellArg (toString action.value)} ]; then
          log_error "expected env ${action.name}=${toString action.value} got='${varRef}'"
          exit 1
        fi
      ''
    else if kind == "fail" then
      "exit ${toString (action.code or 1)}"
    else
      throw "Unsupported action kind: ${kind}";

  renderActionList = actions: lib.concatMapStringsSep "\n" renderAction actions;

  validateActionErrors =
    {
      context,
      index,
      action,
    }:
    let
      prefix = "${context}[${toString index}]";
      kind = action.kind or "";
      hasEnv = action ? env;
      argv = action.argv or [ ];
      args = action.args or [ ];
      knownAppErrs =
        if kind == "runApp" && knownApps != [ ] && !(builtins.elem (action.app or "") knownApps) then
          [
            "${prefix}: app must reference an existing app (got '${action.app or ""}')"
          ]
        else
          [ ];
    in
    if !builtins.isAttrs action then
      [ "${prefix}: action must be an attribute set" ]
    else
      expect (isNonEmptyString kind) "${prefix}: kind is required"
      ++ expect (builtins.elem kind actionKinds) "${prefix}: kind must be one of ${builtins.concatStringsSep ", " actionKinds}"
      ++ expect (
        !hasEnv || isScalarAttrset action.env
      ) "${prefix}: env must be an attrset with shell-safe keys and scalar values"
      ++ (
        if kind == "runApp" then
          expect (isNonEmptyString (action.app or "")) "${prefix}: runApp requires app"
          ++ expect (
            builtins.isList args && isListOfNonEmptyStrings args
          ) "${prefix}: runApp.args must be a list of non-empty strings"
          ++ expect (
            !(action ? passArgs) || builtins.isBool action.passArgs
          ) "${prefix}: runApp.passArgs must be a boolean when set"
          ++ knownAppErrs
        else if kind == "exec" then
          expect (
            builtins.isList argv && isListOfNonEmptyStrings argv && argv != [ ]
          ) "${prefix}: exec.argv must be a non-empty list of non-empty strings"
        else if kind == "logCapture" then
          expect (isArtifactName (
            action.artifact or ""
          )) "${prefix}: logCapture.artifact must match ^[A-Za-z0-9._-]+$"
          ++ expect (
            builtins.isList argv && isListOfNonEmptyStrings argv && argv != [ ]
          ) "${prefix}: logCapture.argv must be a non-empty list of non-empty strings"
        else if kind == "artifactTouch" then
          expect (isArtifactName (
            action.artifact or ""
          )) "${prefix}: artifactTouch.artifact must match ^[A-Za-z0-9._-]+$"
        else if kind == "assertEnvEquals" then
          expect (isEnvVarName (
            action.name or ""
          )) "${prefix}: assertEnvEquals.name must be a shell-safe env var token"
          ++ expect (isScalar (action.value or null)) "${prefix}: assertEnvEquals.value must be a scalar"
        else if kind == "fail" then
          let
            code = action.code or 1;
          in
          expect (
            !(action ? code) || (builtins.isInt code && code > 0 && code < 256)
          ) "${prefix}: fail.code must be an integer in 1..255"
        else
          [ ]
      );

  validateActionListErrors =
    {
      context,
      actions,
      required ? true,
    }:
    if !builtins.isList actions then
      [ "${context}: must be a list of actions" ]
    else
      (expect (!required || actions != [ ]) "${context}: must include at least one action")
      ++ builtins.concatLists (
        lib.imap0 (
          index: action:
          validateActionErrors {
            inherit index action;
            context = context;
          }
        ) actions
      );

  throwActionListViolation =
    {
      context,
      actions,
      required ? true,
    }:
    let
      errs = validateActionListErrors {
        inherit
          context
          actions
          required
          ;
      };
    in
    if errs == [ ] then
      actions
    else
      throw ''
        Nixfied action contract violated for ${context}:
        ${renderErrors errs}
      '';
in
{
  inherit
    actionKinds
    validateActionErrors
    validateActionListErrors
    throwActionListViolation
    renderAction
    renderActionList
    ;
}
