# mfm REST API

The standalone REST binary is deliberately unavailable until a trusted embedding supplies an exact
live `Application`. It does not open a local Store, publish configuration, expose admission/drive
routes, or install a fake provider. Endpoint discovery, credentials, nonce authority, signer
custody, and Runtime topology remain outside this binary and require separately scoped deployment
composition.
