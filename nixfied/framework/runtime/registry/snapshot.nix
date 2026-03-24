{
}:
{
  fromEvents =
    events:
    builtins.foldl' (
      acc: event:
      let
        payload = event.payload or { };
        taskId = payload.taskId or "";
        workflowId = payload.workflowId or "";
        key = if taskId != "" then "task:${taskId}" else "workflow:${workflowId}";
      in
      if taskId == "" && workflowId == "" then
        acc
      else
        acc
        // {
          ${key} = payload.state;
        }
    ) { } events;
}
