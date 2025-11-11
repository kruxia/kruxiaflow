"""Temporal workflow definitions"""

import asyncio
from datetime import timedelta
from temporalio import workflow
from temporalio.common import RetryPolicy
from .activities import echo_activity


@workflow.defn
class SequentialBench5:
    """Sequential workflow with 5 echo activities"""

    @workflow.run
    async def run(self, input_data: dict) -> dict:
        result = input_data
        for i in range(5):
            result = await workflow.execute_activity(
                echo_activity,
                result,
                start_to_close_timeout=timedelta(seconds=10),
            )
        return result


@workflow.defn
class SequentialBench3:
    """Sequential workflow with 3 echo activities"""

    @workflow.run
    async def run(self, input_data: dict) -> dict:
        result = input_data
        for i in range(3):
            result = await workflow.execute_activity(
                echo_activity,
                result,
                start_to_close_timeout=timedelta(seconds=10),
            )
        return result


@workflow.defn
class ParallelBench10:
    """Parallel workflow with 10 echo activities"""

    @workflow.run
    async def run(self, input_data: dict) -> dict:
        # Start activity
        start_result = await workflow.execute_activity(
            echo_activity,
            input_data,
            start_to_close_timeout=timedelta(seconds=10),
        )

        # Parallel activities
        parallel_tasks = [
            workflow.execute_activity(
                echo_activity,
                start_result,
                start_to_close_timeout=timedelta(seconds=10),
            )
            for _ in range(10)
        ]
        parallel_results = await asyncio.gather(*parallel_tasks)

        # End activity (fan-in)
        end_result = await workflow.execute_activity(
            echo_activity,
            {"results": parallel_results},
            start_to_close_timeout=timedelta(seconds=10),
        )

        return end_result
