{
}:
{
  fromEvents =
    events:
    builtins.foldl' (
      acc: event:
      let
        taskId = event.taskId or "";
        workflowId = event.workflowId or "";
        key = if taskId != "" then "task:${taskId}" else "workflow:${workflowId}";
      in
      if taskId == "" && workflowId == "" then
        acc
      else
        acc
        // {
          ${key} = event.state;
        }
    ) { } events;
}
