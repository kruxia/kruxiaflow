"""Airflow DAG definitions for benchmarking"""

from datetime import datetime
from airflow import DAG
from operators import EchoOperator


default_args = {
    "owner": "benchmark",
    "depends_on_past": False,
    "start_date": datetime(2024, 1, 1),
    "email_on_failure": False,
    "email_on_retry": False,
    "retries": 0,
}


# Sequential benchmark with 5 activities
with DAG(
    dag_id="sequential_bench_5",
    default_args=default_args,
    schedule_interval=None,
    catchup=False,
    max_active_runs=1,
) as sequential_bench_5:
    tasks = []
    for i in range(5):
        task = EchoOperator(
            task_id=f"echo_{i}",
            input_data={"activity": i},
        )
        if tasks:
            tasks[-1] >> task
        tasks.append(task)


# Sequential benchmark with 3 activities
with DAG(
    dag_id="sequential_bench_3",
    default_args=default_args,
    schedule_interval=None,
    catchup=False,
    max_active_runs=1,
) as sequential_bench_3:
    tasks = []
    for i in range(3):
        task = EchoOperator(
            task_id=f"echo_{i}",
            input_data={"activity": i},
        )
        if tasks:
            tasks[-1] >> task
        tasks.append(task)


# Parallel benchmark with 10 activities
with DAG(
    dag_id="parallel_bench_10",
    default_args=default_args,
    schedule_interval=None,
    catchup=False,
    max_active_runs=1,
) as parallel_bench_10:
    start = EchoOperator(
        task_id="start",
        input_data={"stage": "start"},
    )

    parallel_tasks = []
    for i in range(10):
        task = EchoOperator(
            task_id=f"parallel_{i}",
            input_data={"activity": i},
        )
        start >> task
        parallel_tasks.append(task)

    end = EchoOperator(
        task_id="end",
        input_data={"stage": "end"},
    )

    for task in parallel_tasks:
        task >> end
