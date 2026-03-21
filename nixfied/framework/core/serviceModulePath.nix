serviceName:
if serviceName == "postgres" then
  ../runtime/services/postgres/default.nix
else if serviceName == "nginx" then
  ../runtime/services/nginx/default.nix
else if serviceName == "minio" then
  ../runtime/services/minio/default.nix
else if serviceName == "reth" then
  ../runtime/services/reth/default.nix
else if serviceName == "helios" then
  ../runtime/services/helios/default.nix
else
  throw "unsupported runtime service '${serviceName}'"
