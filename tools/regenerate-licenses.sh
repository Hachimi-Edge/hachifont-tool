#!/usr/bin/env bash

# cargo install --locked --features cli cargo-about

cargo about generate tools/about.hbs -o public/licenses.html
