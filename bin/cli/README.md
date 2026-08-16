# MFM CLI

The standalone CLI currently reports build/version metadata only. It does not construct a live
Runtime or expose run progression. A future start route must accept and echo an explicit RunId,
select a typed entry point, and call Application; it must not invent provider defaults or generic
execution DTOs.
