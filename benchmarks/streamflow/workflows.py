"""StreamFlow workflow definitions as Python dicts (JSON-serializable)"""


def create_sequential_workflow(num_activities: int) -> dict:
    """Create sequential workflow with N echo activities"""
    activities = []

    for i in range(num_activities):
        activity = {
            "key": f"activity_{i}",
            "namespace": "default",
            "name": "echo",
            "parameters": {},
        }

        # Add 'following' relationship (except for last activity)
        if i < num_activities - 1:
            activity["following"] = [{
                "activity_key": f"activity_{i + 1}",
                "conditions": None,
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
            "namespace": "default",
            "name": "echo",
            "parameters": {},
            "following": [
                {"activity_key": f"parallel_{i}", "conditions": None}
                for i in range(num_parallel)
            ],
        }
    ]

    # Parallel activities
    for i in range(num_parallel):
        activities.append({
            "key": f"parallel_{i}",
            "namespace": "default",
            "name": "echo",
            "parameters": {},
            "preceding": [{"activity_key": "start", "conditions": None}],
            "following": [{"activity_key": "end", "conditions": None}],
        })

    # End activity (fan-in)
    activities.append({
        "key": "end",
        "namespace": "default",
        "name": "echo",
        "parameters": {},
        "preceding": [
            {"activity_key": f"parallel_{i}", "conditions": None}
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
