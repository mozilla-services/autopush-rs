#!/bin/bash
# Generate the mdBook version of the document
echo Generating the cargo docs
cargo doc --all-features --workspace --no-deps
echo Generating mdbook
mdbook-mermaid install .
mdbook build
echo Generate the API docs
mkdir -p output/api
cargo doc --all-features --workspace --no-deps
# bring just the built docs over into the artifact (there's a lot of build
# detrius)
cp -r ../target/doc/* output/api
