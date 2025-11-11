"""Temporal activities"""

from temporalio import activity


@activity.defn
async def echo_activity(input_data: dict) -> dict:
    """Simple echo activity matching StreamFlow's echo"""
    return input_data
