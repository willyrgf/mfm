# mfm-keystore

`Keystore` is thread-affine and neither `Send` nor `Sync`. Secret material is held in zeroizing
owners and never appears in persisted, diagnostic, or public surfaces. Async integration must use a
dedicated owner and bounded command handle.
