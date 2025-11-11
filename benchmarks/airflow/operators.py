"""Custom Airflow operator for echo activity"""

from airflow.models import BaseOperator


class EchoOperator(BaseOperator):
    """Simple echo operator that returns its input"""

    def __init__(self, input_data=None, **kwargs):
        super().__init__(**kwargs)
        self.input_data = input_data or {}

    def execute(self, context):
        """Execute the echo operation"""
        return self.input_data
