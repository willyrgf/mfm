# Isolation runner schema validation and normalization.
{
  pkgs,
  knownApps ? [ ],
}:

let
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    expect
    renderErrors
    isListOfNonEmptyStrings
    isScalarAttrset
    ;
  actionSchema = import ./action-schema.nix {
    inherit pkgs knownApps;
  };

  validateIsolation =
    {
      isolation,
      envNames,
      slotMax,
    }:
    let
      configuredSlots = isolation.slots or [ ];
      configuredEnvs = isolation.envs or [ ];
      runEnv = isolation.runEnv or { };
      runAction =
        if isolation ? run then
          isolation.run
        else
          {
            kind = "runApp";
            app = "ci";
            args = [ "--summary" ];
          };
      validateAction =
        if isolation ? validate then
          isolation.validate
        else
          {
            kind = "runApp";
            app = "validate-env";
          };
      setupActions = isolation.setupActions or [ ];
      cleanupActions = isolation.cleanupActions or [ ];

      slotErrs = builtins.concatLists (
        map (
          slot:
          expect (
            builtins.isInt slot && slot >= 0 && slot <= slotMax
          ) "isolation.slots values must be integers 0-${toString slotMax} (got ${toString slot})"
        ) configuredSlots
      );
      envErrs = builtins.concatLists (
        map (
          env:
          expect (
            builtins.elem env envNames
          ) "isolation.envs contains unknown environment '${env}' (expected one of: ${builtins.concatStringsSep ", " envNames})"
        ) configuredEnvs
      );

      errs =
        expect (!(isolation ? runCommand)) "isolation.runCommand has been removed. Use isolation.run action."
        ++ expect (!(isolation ? validateCommand)) "isolation.validateCommand has been removed. Use isolation.validate action."
        ++ expect (!(isolation ? preInstall)) "isolation.preInstall has been removed. Use isolation.setupActions."
        ++ expect (!(isolation ? cleanup)) "isolation.cleanup has been removed. Use isolation.cleanupActions."
        ++ expect (!(isolation ? runApp)) "isolation.runApp has been removed. Use isolation.run = { kind = \"runApp\"; ... }."
        ++ expect (!(isolation ? runArgs)) "isolation.runArgs has been removed. Use isolation.run.args."
        ++ expect (
          !(isolation ? slots) || (builtins.isList configuredSlots)
        ) "isolation.slots must be a list when set"
        ++ expect (
          !(isolation ? envs) || (builtins.isList configuredEnvs && isListOfNonEmptyStrings configuredEnvs)
        ) "isolation.envs must be a list of non-empty strings when set"
        ++ expect (
          isScalarAttrset runEnv
        ) "isolation.runEnv must be an attrset with shell-safe keys and scalar values"
        ++ slotErrs
        ++ envErrs
        ++ actionSchema.validateActionListErrors {
          context = "isolation.run";
          actions = [ runAction ];
          required = true;
        }
        ++ actionSchema.validateActionListErrors {
          context = "isolation.validate";
          actions = [ validateAction ];
          required = true;
        }
        ++ actionSchema.validateActionListErrors {
          context = "isolation.setupActions";
          actions = setupActions;
          required = false;
        }
        ++ actionSchema.validateActionListErrors {
          context = "isolation.cleanupActions";
          actions = cleanupActions;
          required = false;
        };
    in
    if errs == [ ] then
      isolation
      // {
        runScript = actionSchema.renderAction runAction;
        validateScript = actionSchema.renderAction validateAction;
        setupScript = actionSchema.renderActionList setupActions;
        cleanupScript = actionSchema.renderActionList cleanupActions;
      }
    else
      throw ''
        Nixfied isolation config violated:
        ${renderErrors errs}

        Use typed isolation fields:
          - isolation.run / isolation.validate
          - isolation.setupActions / isolation.cleanupActions
      '';
in
{
  inherit validateIsolation;
}
