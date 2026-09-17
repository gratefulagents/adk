module durable-postgres-interop

go 1.26.2

require (
	github.com/gratefulagents/sdk v0.0.0
	github.com/lib/pq v1.10.9
)

require github.com/google/uuid v1.6.0 // indirect

replace github.com/gratefulagents/sdk => ../../../repos/sdk
