# Get a list of all tasks
list:
  @just --list --unsorted

# Setup local development services
up: db cluster

# Remove all local development services
down:
  docker compose down --volumes
  k3d cluster delete external-postgres

# Launch the database
db:
  docker compose up -d

# Launch a development cluster with k3d
cluster:
  k3d cluster create external-postgres --servers 1 --agents 1 --registry-create kube
