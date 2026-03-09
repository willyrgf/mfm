# Installer app for the framework
{
  pkgs,
  lib,
  frameworkRoot,
  frameworkRevision ? "unknown",
}:

let
  inherit (lib.appApi) mkNixfiedApp;
  installManifest = import ./install-manifest.nix { inherit pkgs; };
  projectTemplates = installManifest.projectTemplates;
  frameworkHelpers = installManifest.frameworkHelpers;
  optionalTemplates = builtins.filter (t: !(t.required or false)) projectTemplates;
  templateFilterPlanDataJson = builtins.toJSON installManifest.templateFilterPlanData;
  filterHelpValues = builtins.concatStringsSep "," installManifest.templateFilterDisplayTokens;
  filterAliasNotes = builtins.concatStringsSep "; " (
    builtins.concatLists (
      map (t: map (alias: "${alias} is an alias of ${t.file}") (t.aliases or [ ])) projectTemplates
    )
  );
  filteredTemplateHint = builtins.concatStringsSep "," (
    map (t: pkgs.lib.strings.removeSuffix ".nix" t.file) optionalTemplates
  );
  promptPlanScript = import ./prompt-plan.nix {
    inherit
      pkgs
      lib
      frameworkRoot
      ;
  };
  installRuntime = import ./install-runtime.nix {
    inherit
      pkgs
      filterHelpValues
      filterAliasNotes
      frameworkRevision
      frameworkRoot
      promptPlanScript
      templateFilterPlanDataJson
      ;
  };

  installScript = ''
    ${lib.loggingPrelude}

    set -euo pipefail

    source ${installRuntime}

    parse_install_args "$@"
    resolve_install_repo_root
    maybe_reenter_install_worktree
    resolve_install_branch_context

    SRC="${frameworkRoot}"
    resolve_framework_revision

                                    PREV_FRAMEWORK_REVISION="unknown"
                                    if [ "$MODE" = "upgrade" ] && [ -f "$ROOT/nixfied/VENDORED.txt" ]; then
                                      PREV_FRAMEWORK_REVISION=$(${pkgs.gawk}/bin/awk '
                                        $0 ~ /^Framework source revision \(install\/upgrade\):$/ { in_section=1; next }
                                        in_section && $0 ~ /^[[:space:]]*-[[:space:]]+/ {
                                          line=$0
                                          sub(/^[[:space:]]*-[[:space:]]+/, "", line)
                                          print line
                                          exit
                                        }
                                      ' "$ROOT/nixfied/VENDORED.txt" 2>/dev/null || true)
                                      if [ -z "$PREV_FRAMEWORK_REVISION" ]; then
                                        PREV_FRAMEWORK_REVISION="unknown"
                                      fi
                                      log_info "Existing vendored revision rev=$PREV_FRAMEWORK_REVISION"
                                    fi

        	                    	        if [ ! -f "$SRC/flake.nix" ] || [ ! -d "$SRC/nixfied" ]; then
        	                    	          log_error "Framework source is missing required files."
        	                    	          exit 1
                            	        fi

    maybe_confirm_install_overwrite

                            	        log_info "Installing framework files"

                                    cp -f "$SRC/flake.nix" "$ROOT/flake.nix"
                                    if [ -f "$SRC/flake.lock" ]; then
                                      cp -f "$SRC/flake.lock" "$ROOT/flake.lock"
                                    fi

                            	        PRESERVE_PROJECT=false
                            	        if [ -d "$ROOT/nixfied/project" ] && [ "$RESET_PROJECT" != "true" ]; then
                            	          PRESERVE_PROJECT=true
                            	        fi
                            	        if [ "$MODE" = "upgrade" ] && [ "$RESET_PROJECT" != "true" ]; then
                            	          PRESERVE_PROJECT=true
                            	        fi
                            	
                            	        PRESERVE_LOCAL=false
                            	        if [ -d "$ROOT/nixfied/local" ]; then
                            	          PRESERVE_LOCAL=true
                            	        fi

                            		        if [ -n "$FILTERS_RAW" ] && [ "$PRESERVE_PROJECT" = "true" ]; then
                            		          log_info "Skipping --filter on upgrade (nixfied/project is preserved)."
                            		          FILTERS_RAW=""
                            		        fi

                                    if [ -d "$ROOT/nixfied" ]; then
                                      chmod -R u+w "$ROOT/nixfied" 2>/dev/null || true
                                    fi

                                    # Ensure the .framework path is a directory in the current layout.
                                    if [ -f "$ROOT/nixfied/.framework" ]; then
                                      rm -f "$ROOT/nixfied/.framework"
                                    fi

                            	        RSYNC_EXCLUDES=()
                            	        PRESERVE_MSG=""
                            	        if [ "$PRESERVE_PROJECT" = "true" ]; then
                            	          RSYNC_EXCLUDES+=(--exclude='/project/')
                            	          PRESERVE_MSG="nixfied/project/"
                            	        fi
                            	        if [ "$PRESERVE_LOCAL" = "true" ]; then
                            	          RSYNC_EXCLUDES+=(--exclude='/local/')
                            	          if [ -n "$PRESERVE_MSG" ]; then
                            	            PRESERVE_MSG="$PRESERVE_MSG and nixfied/local/"
                            	          else
                            	            PRESERVE_MSG="nixfied/local/"
                            	          fi
                            	        fi
                            	
            	                		        if [ "''${#RSYNC_EXCLUDES[@]}" -gt 0 ]; then
            	                		          log_info "Upgrading nixfied/ (preserving $PRESERVE_MSG)"
            	                		          ${pkgs.rsync}/bin/rsync -a --delete --chmod=Du+w,Fu+w "''${RSYNC_EXCLUDES[@]}" "$SRC/nixfied/" "$ROOT/nixfied/"
            	                		        else
            	                		          ${pkgs.rsync}/bin/rsync -a --delete --chmod=Du+w,Fu+w "$SRC/nixfied/" "$ROOT/nixfied/"
            	                		        fi

                                    if [ -f "$SRC/README.md" ]; then
                                      cp -f "$SRC/README.md" "$ROOT/nixfied/README.md"
                                    fi

                                    {
                                      echo "Vendored Framework"
                                      echo "=================="
                                      echo ""
                                      echo 'This repository vendors the Nixfied framework under `nixfied/`.'
                                      echo ""
                                      echo "Framework source revision (install/upgrade):"
                                      echo "- $FRAMEWORK_REVISION"
                                      echo ""
                                      echo "Framework-owned (overwritten on framework::upgrade):"
                                      echo "- flake.nix, flake.lock"
                                      echo "- nixfied/.framework/"
                                      echo ""
                                      echo "User-owned (preserved on framework::upgrade by default):"
                                      echo "- nixfied/project/ (primary customization surface)"
                                      echo "- nixfied/local/ (extensions: extra apps/packages/devShells)"
                                      echo ""
                                      echo "If you need to customize behavior, prefer editing files under nixfied/project/"
                                      echo "and nixfied/local/ rather than editing framework code."
                                    } > "$ROOT/nixfied/VENDORED.txt"
                                    rm -f "$ROOT/nixfied/UPGRADE_CHECK.txt" 2>/dev/null || true

                                    chmod -R u+w "$ROOT/nixfied" 2>/dev/null || true
                                    if command -v chflags >/dev/null 2>&1; then
                                      chflags -R nouchg "$ROOT/nixfied" 2>/dev/null || true
                                    fi
                                    if command -v chattr >/dev/null 2>&1; then
                                      chattr -R -i "$ROOT/nixfied" 2>/dev/null || true
                                    fi
                                    mkdir -p "$ROOT/nixfied/.framework"
                                    chmod -R u+w "$ROOT/nixfied/.framework" 2>/dev/null || true
                                    if command -v chflags >/dev/null 2>&1; then
                                      chflags -R nouchg "$ROOT/nixfied/.framework" 2>/dev/null || true
                                    fi
                                    if command -v chattr >/dev/null 2>&1; then
                                      chattr -R -i "$ROOT/nixfied/.framework" 2>/dev/null || true
                                    fi
                                    rm -f "$ROOT/nixfied/.framework/.workspace"

                                    if [ -n "$FILTERS_RAW" ]; then
                                      FILTER_PLAN_JSON="$(compute_template_filter_plan "$FILTERS_RAW")"
                                      FIRST_UNKNOWN_FILTER="$(
                                        printf '%s' "$FILTER_PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.unknown[0] // empty'
                                      )"
                                      if [ -n "$FIRST_UNKNOWN_FILTER" ]; then
                                        log_error "Unknown filter: $FIRST_UNKNOWN_FILTER"
                                        exit 1
                                      fi

                                      while IFS= read -r template_file; do
                                        if [ -n "$template_file" ]; then
                                          rm -f "$ROOT/nixfied/project/$template_file" 2>/dev/null || true
                                        fi
                                      done < <(
                                        printf '%s' "$FILTER_PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.pruneFiles[]?'
                                      )

                                      printf '%s' "$FILTER_PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.defaultNix' > "$ROOT/nixfied/project/default.nix"
                                    fi

    maybe_generate_prompt_plan

                            		        if [ "$PRESERVE_PROJECT" = "true" ] || [ "$PRESERVE_LOCAL" = "true" ] || [ "$MODE" = "upgrade" ]; then
                            		          log_ok "Framework upgraded."
                            		          echo "    Preserved nixfied/project/ and nixfied/local/ (pass --reset-project to overwrite project templates)."
                            		        else
                            		          log_ok "Framework installed."
                                    fi
                                    echo "Next:"
                                    echo "  - Edit nixfied/project/conf.nix"
                                    echo "  - Customize nixfied/project/{${filteredTemplateHint}}.nix (prod.nix defines the build command)"
  '';

  helperScriptFor =
    helper:
    if helper.runner == "install" then
      installScript
    else if helper.runner == "prompt-plan" then
      ''
        ${promptPlanScript} "$@"
      ''
    else
      throw "Unknown framework helper runner: ${helper.runner}";

  mkFrameworkHelperApp =
    helper:
    mkNixfiedApp {
      name = helper.name;
      api = helper.api;
      env = helper.env or { };
      useDeps = false;
      script = helperScriptFor helper;
    };
in
builtins.listToAttrs (
  map (helper: {
    name = helper.name;
    value = mkFrameworkHelperApp helper;
  }) frameworkHelpers
)
