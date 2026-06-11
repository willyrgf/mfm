{ adapters, ... }:
{
  # The synthetic adapter is a runnable starter service. Replace it with your
  # own service/task declarations or another adapter (e.g. adapters.postgres).
  imports = [ adapters.synthetic ];

  nixfied.project.projectId = "mfm";
  nixfied.project.name = "MFM";
  nixfied.codebases.main.logicalRoot = ".";

  nixfied.workflows.test = {
    servicesRequired = [ "synthetic" ];
    nodes = [
      {
        nodeId = "smoke";
        taskId = "smoke";
      }
    ];
  };
}
