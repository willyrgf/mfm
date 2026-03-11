# Package Helios from upstream source.
{
  lib,
  fetchFromGitHub,
  rustPlatform,
  pkg-config,
  perl,
}:

rustPlatform.buildRustPackage rec {
  pname = "helios";
  version = "unstable-2026-02-04";

  src = fetchFromGitHub {
    owner = "a16z";
    repo = "helios";
    rev = "4a32ac1a9fbcf46386a497e4e0a7232ad1388762";
    hash = "sha256-AJps+uQrN2fvtT78TsNaRiUtM+GaiPVBHfWumyNzt5M=";
  };

  cargoHash = "sha256-RSTwadwdmZ35RwIjsomIjFdsvdayAxP13Y6GzXTJBQI=";

  patches = [
    ./patches/0001-disable-reqwest-hickory-dns.patch
    ./patches/0002-limit-light-client-updates-request.patch
  ];

  cargoBuildFlags = [
    "--package"
    "helios-cli"
    "--bin"
    "helios"
  ];

  cargoTestFlags = cargoBuildFlags;
  doCheck = false;

  nativeBuildInputs = [
    pkg-config
    perl
  ];

  meta = with lib; {
    description = "Ethereum light client";
    homepage = "https://github.com/a16z/helios";
    license = licenses.mit;
    maintainers = [ ];
    mainProgram = "helios";
    platforms = platforms.unix;
  };
}
