# MFM REST API

The standalone REST binary currently prints one unavailable diagnostic and exits. It binds no
listener, exposes no metadata or health route, and has no trusted live Runtime composition or
run-progression endpoint. Any future route must use explicit RunId, typed Application input/output,
bounded request parsing, and redaction-safe errors.
