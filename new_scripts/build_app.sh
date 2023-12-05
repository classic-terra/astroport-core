#!/usr/bin/env bash

set -e

projectPath=$(pwd)

cd "$projectPath/new_scripts" && node --loader ts-node/esm create_astro.ts
# cd "$projectPath/scripts" && node --loader ts-node/esm deploy_core.ts
# cd "$projectPath/scripts" && node --loader ts-node/esm deploy_generator.ts
# cd "$projectPath/scripts" && node --loader ts-node/esm deploy_pools_testnet.ts
