{
  pkgs ? null,
}:
{
  fromEvents =
    events:
    builtins.foldl' (
      acc: event:
      let
        key =
          if (event.taskId or "") != "" then
            "task:${event.taskId}"
          else
            "workflow:${event.workflowId or "unknown"}";
      in
      acc
      // {
        ${key} = event.state;
      }
    ) { } events;
}
