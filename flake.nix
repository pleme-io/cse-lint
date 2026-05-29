{
  description = "cse-lint — Constructive Substrate Engineering audit linter";

  inputs.substrate.url = "github:pleme-io/substrate";

  outputs = { substrate, ... }: substrate.rust.tool { src = ./.; };
}
