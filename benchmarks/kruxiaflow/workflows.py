"""Kruxia Flow workflow definitions as Python dicts (JSON-serializable)"""


# Echo script for py-std worker (equivalent to std.echo)
PY_ECHO_SCRIPT = "OUTPUT = INPUT"


def _echo_activity(worker: str) -> dict:
    """Create an echo activity definition for the given worker type."""
    if worker == "std":
        return {"worker": "std", "activity_name": "echo", "parameters": {}}
    else:
        return {
            "worker": worker,
            "activity_name": "script",
            "parameters": {"script": PY_ECHO_SCRIPT, "inputs": {}},
        }


def create_sequential_workflow(num_activities: int, worker: str = "std") -> dict:
    """Create sequential workflow with N echo activities"""
    suffix = f"_{worker}" if worker != "std" else ""
    activities = []

    for i in range(num_activities):
        activity = {"key": f"activity_{i}", **_echo_activity(worker)}

        # Add 'dependency_of' relationship (except for last activity)
        if i < num_activities - 1:
            activity["dependency_of"] = [{
                "activity_key": f"activity_{i + 1}",
            }]

        activities.append(activity)

    return {
        "name": f"sequential_bench_{num_activities}{suffix}",
        "activities": activities,
    }


def create_parallel_workflow(num_parallel: int, worker: str = "std") -> dict:
    """Create parallel workflow with fan-out and fan-in"""
    suffix = f"_{worker}" if worker != "std" else ""
    activities = [
        # Start activity (fans out)
        {
            "key": "start",
            **_echo_activity(worker),
            "dependency_of": [
                {"activity_key": f"parallel_{i}"}
                for i in range(num_parallel)
            ],
        }
    ]

    # Parallel activities
    for i in range(num_parallel):
        activities.append({
            "key": f"parallel_{i}",
            **_echo_activity(worker),
            "depends_on": [{"activity_key": "start"}],
            "dependency_of": [{"activity_key": "end"}],
        })

    # End activity (fan-in)
    activities.append({
        "key": "end",
        **_echo_activity(worker),
        "depends_on": [
            {"activity_key": f"parallel_{i}"}
            for i in range(num_parallel)
        ],
    })

    return {
        "name": f"parallel_bench_{num_parallel}{suffix}",
        "activities": activities,
    }


# Pre-defined workflows: std worker
SEQUENTIAL_5 = create_sequential_workflow(5)
SEQUENTIAL_3 = create_sequential_workflow(3)
PARALLEL_10 = create_parallel_workflow(10)

# Pre-defined workflows: py-std worker
PY_STD_SEQUENTIAL_5 = create_sequential_workflow(5, worker="py-std")
PY_STD_SEQUENTIAL_3 = create_sequential_workflow(3, worker="py-std")
PY_STD_PARALLEL_10 = create_parallel_workflow(10, worker="py-std")
