2025-10-14

# Architecture

The StreamFlow system has the following parts:

- Server:
    - API that is used to kick-off workflows and to query the status of things.
    - Schedule workflows.
    - Subscribes to the events that come from workflows so as to know at any time what is the current state of the system.

- Worker:
    - Workflow decision: Decide what to do next, kick off the next activity/ies,
      handle response
    - Activity: Function: do the work, return results.
    - Publish the state of the workflow after every state change
    - Pick up new and hung workflows (i.e., those for which the lease has expired)

- Workflow:
    - 0 or more Activities, each with 
        - a state (unstarted, started, done, failed)
        - inputs (required and optional)
        - outputs
    - Edges (forming a graph) showing the relationships between activities
        - Edges are directed (so it's a directed graph)
        - Edges can create cycles, but every path must be able to end
        - Edges can be conditional based on the state or outputs of the preceding Activity/ies or on the state of the Workflow
            - for example: The previous Activity failed 1 time => cycle, 3 times => don't cycle.

- Persistent data:
    - Workflow state events
    - Activity queue
    - Workflow data storage (shared among activities in this Workflow run)

Process:

- (HTTP) Client posts Workflow request to API Server.
- API Server publishes Workflow created event.
- Workflow Orchestrator: 
    - reads the Workflow created/updated event
        - if Workflow created and storage needed: initializes Workflow data storage
    - decides the next Activity/ies based on the Workflow state (edges and activities)
    - schedules the next Activity/ies on the Activity Queue (idempotently: if not already scheduled)
    - publishes the updated Workflow state event
- Activity Worker:
    - queries the Activity Queue, takes an Activity, and POSTs Activity started event to the API Server
        - 200 OK => processes the Activity => completed/failed
        - 409 Conflict => not valid, canceled
    - completes the Activity from the Queue and POSTs the Activity status event
- API Server:
    - receives Activity started / completed / failed / canceled event
    - reads Workflow status and (idempotently) publishes the Workflow updated event (using optimistic lock on workflow step)
