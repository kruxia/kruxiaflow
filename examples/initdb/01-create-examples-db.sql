-- Create the kruxiaflow_examples database for running example workflows.
-- This script runs automatically on first postgres startup via
-- docker-entrypoint-initdb.d when using docker-compose.override.yml.

CREATE DATABASE kruxiaflow_examples;
