{ lib, model }:
import ../../compiler/compile-selection-index.nix { inherit lib; } {
  tasks = model.tasks or { };
  workflows = model.workflows or { };
  serviceCatalog = model.serviceCatalog or { };
}
