"""Activity registry and executor.

Mirrors Rust ActivityRegistry for interface compatibility.
"""

import asyncio
from typing import Any

from .activity import Activity, ActivityResult
from .context import ActivityContext
from .errors import ActivityNotFoundError, ActivityTimeoutError


class ActivityRegistry:
    """
    Activity registry and executor.

    Mirrors Rust ActivityRegistry for interface compatibility.
    """

    def __init__(self) -> None:
        self._activities: dict[str, Activity] = {}

    def register(self, activity: Activity, worker: str) -> None:
        """
        Register an activity implementation for a worker type.

        The worker type is provided at registration time, not defined
        on the activity itself. This allows the same activity to be
        registered with different worker types if needed.

        Key format: "{worker}.{name}"

        Args:
            activity: The activity implementation
            worker: Worker type (e.g., "python", "custom")
        """
        key = f"{worker}.{activity.name}"
        self._activities[key] = activity

    def get(self, worker: str, name: str) -> Activity | None:
        """Get activity by worker and name."""
        key = f"{worker}.{name}"
        return self._activities.get(key)

    def activity_types(self) -> list[str]:
        """Get all registered activity types."""
        return list(self._activities.keys())

    async def execute(
        self,
        worker: str,
        name: str,
        params: dict[str, Any],
        ctx: ActivityContext,
        timeout: float,
    ) -> ActivityResult:
        """
        Execute an activity with timeout.

        Args:
            worker: Worker type
            name: Activity name
            params: Input parameters
            ctx: Execution context
            timeout: Timeout in seconds

        Returns:
            ActivityResult on success

        Raises:
            ActivityNotFoundError: Activity not found
            ActivityTimeoutError: Execution timed out
            Exception: Activity execution failed
        """
        key = f"{worker}.{name}"
        activity = self._activities.get(key)

        if not activity:
            raise ActivityNotFoundError(f"Activity implementation not found: {key}")

        # Execute with timeout
        try:
            return await asyncio.wait_for(
                activity.execute(params, ctx),
                timeout=timeout,
            )
        except asyncio.TimeoutError as exc:
            raise ActivityTimeoutError(
                f"Activity execution timed out after {timeout}s"
            ) from exc
