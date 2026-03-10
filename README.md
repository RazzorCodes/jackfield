Binary-native message bus with affinity-based routing between registered components. It follows a fire-and-forget model: senders dispatch and move on, Jackfield handles the rest.
Routing decisions are driven by consumption history and sender-defined blacklists: receivers that have previously consumed a given message type are ranked higher in subsequent lookups, so over time the right consumers are reached with minimal traversal.
The initial implementation targets in-process zero-copy message passing, with inter-service routing (across microservices) as a planned extension.
