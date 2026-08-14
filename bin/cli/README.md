# mfm CLI

The standalone CLI exposes only local help/version metadata. It has no trusted live composition:
endpoint discovery, credentials, nonce authority, signer custody, and the finite Runtime registry
belong to a separately supplied embedding. It therefore does not admit, drive, replay, or export
actionable EVM or Portfolio runs, and it never installs a fake provider or local Store.
