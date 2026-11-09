#!/bin/sh
# Registers the namespace the benchmark uses once the server answers. Runs from
# the temporalio/admin-tools image after the server is healthy.
set -eu

NAMESPACE=${DEFAULT_NAMESPACE:-default}
ADDRESS=${TEMPORAL_ADDRESS:-temporal:7233}

echo 'Waiting for the Temporal server...'
until temporal operator cluster health --address "$ADDRESS" > /dev/null 2>&1; do sleep 1; done

if temporal operator namespace describe -n "$NAMESPACE" --address "$ADDRESS" > /dev/null 2>&1; then
  echo "Namespace '$NAMESPACE' already exists"
else
  temporal operator namespace create -n "$NAMESPACE" --address "$ADDRESS"
  echo "Namespace '$NAMESPACE' created"
fi
