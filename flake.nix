{
  description = "cse-lint — Constructive Substrate Engineering audit linter";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-25.11";
    crate2nix.url = "github:nix-community/crate2nix";
    flake-utils.url = "github:numtide/flake-utils";
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crate2nix, flake-utils, substrate, ... }:
    (import "${substrate}/lib/rust-tool-release-flake.nix" {
      inherit nixpkgs crate2nix flake-utils;
    }) {
      toolName = "cse-lint";
      src = self;
      repo = "pleme-io/cse-lint";
      module = {
        description = "Constructive Substrate Engineering audit linter — measures CSE adherence across the pleme-io fleet";
        # cse-lint is a CLI; no daemon, no MCP shim, no HTTP service.
        # Just `cse-lint audit <path>` and `cse-lint report --json`.
      };
    };
}
