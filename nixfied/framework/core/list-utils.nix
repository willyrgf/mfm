let
  uniquePreserveOrder =
    list:
    builtins.foldl' (acc: value: if builtins.elem value acc then acc else acc ++ [ value ]) [ ] list;
in
{
  inherit uniquePreserveOrder;

  uniqueNonEmptyPreserveOrder =
    list: uniquePreserveOrder (builtins.filter (value: value != null && value != "") list);
}
