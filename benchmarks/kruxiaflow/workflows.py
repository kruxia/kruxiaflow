"""Kruxia Flow workflow definitions as Python dicts (JSON-serializable)"""


def create_sequential_workflow(num_activities: int) -> dict:
    """Create sequential workflow with N echo activities"""
    activities = []

    for i in range(num_activities):
        activity = {
            "key": f"activity_{i}",
            "worker": "builtin",
            "activity_name": "echo",
            "parameters": {},
        }

        # Add 'dependency_of' relationship (except for last activity)
        if i < num_activities - 1:
            activity["dependency_of"] = [{
                "activity_key": f"activity_{i + 1}",
            }]

        activities.append(activity)

    return {
        "name": f"sequential_bench_{num_activities}",
        "activities": activities,
    }


def create_parallel_workflow(num_parallel: int) -> dict:
    """Create parallel workflow with fan-out and fan-in"""
    activities = [
        # Start activity (fans out)
        {
            "key": "start",
            "worker": "builtin",
            "activity_name": "echo",
            "parameters": {},
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
            "worker": "builtin",
            "activity_name": "echo",
            "parameters": {},
            "depends_on": [{"activity_key": "start"}],
            "dependency_of": [{"activity_key": "end"}],
        })

    # End activity (fan-in)
    activities.append({
        "key": "end",
        "worker": "builtin",
        "activity_name": "echo",
        "parameters": {},
        "depends_on": [
            {"activity_key": f"parallel_{i}"}
            for i in range(num_parallel)
        ],
    })

    return {
        "name": f"parallel_bench_{num_parallel}",
        "activities": activities,
    }


# Pre-defined workflows matching internal benchmarks
SEQUENTIAL_5 = create_sequential_workflow(5)
SEQUENTIAL_3 = create_sequential_workflow(3)
PARALLEL_10 = create_parallel_workflow(10)
