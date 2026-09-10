# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from __future__ import annotations

from typing import NoReturn
from uuid import UUID

from dynwinrt import RoApartment, projected_lifetime_scope, release_projected
from generated.windows.application_model.background import (
    IBackgroundTask,
    IBackgroundTaskInstance,
)
from generated.windows.foundation import IClosable, IStringable


class TaskInstance:
    def __init__(self) -> None:
        self.progress = 0

    def get_instance_id(self) -> UUID:
        return UUID("4ab7eea7-29a1-4f48-a6da-22894e210ec0")

    def get_progress(self) -> int:
        return self.progress

    def set_progress(self, value: int) -> None:
        self.progress = value

    def get_suspended_count(self) -> int:
        return 0

    def get_task(self) -> NoReturn:
        raise NotImplementedError("This fixture has no registered OS task")

    def get_trigger_details(self) -> NoReturn:
        raise NotImplementedError("This fixture has no OS trigger")

    def add_canceled(self, handler: object) -> NoReturn:
        raise NotImplementedError("This fixture has no cancellation source")

    def remove_canceled(self, token: object) -> NoReturn:
        raise NotImplementedError("This fixture has no cancellation source")

    def get_deferral(self) -> NoReturn:
        raise NotImplementedError("This fixture does not create OS deferrals")


class Task:
    def __init__(self) -> None:
        self.runs = 0
        self.closes = 0

    def run(self, instance: IBackgroundTaskInstance | None) -> None:
        if instance is None:
            raise ValueError("A task instance is required by this handler")
        instance.progress = instance.progress + 1
        self.runs += 1

    def to_string(self) -> str:
        return f"Python task: {self.runs} native Run calls"

    def close(self) -> None:
        self.closes += 1


def main() -> None:
    with RoApartment(1), projected_lifetime_scope():
        handlers = Task()
        with IBackgroundTaskInstance.implement(TaskInstance()) as instance_impl, IBackgroundTask.implement(
            handlers, interfaces=[(IStringable, handlers), (IClosable, handlers)]
        ) as impl:
            # Primary views are owned by the handles and require no separate release.
            assert impl.value is impl.value
            impl.value.run(instance_impl.value)
            assert instance_impl.value.progress == 1
            text = IStringable.from_implementation(impl)
            duplicate_text = IStringable.from_implementation(impl)
            closable = IClosable.from_implementation(impl)
            try:
                assert text is not duplicate_text
                release_projected(duplicate_text)
                print(text.to_string())
                independent_task = IBackgroundTask.from_implementation(impl)
                impl.release()
                independent_task.run(instance_impl.value)
                release_projected(independent_task)
                assert instance_impl.value.progress == 2
                closable.close()
                assert handlers.closes == 1
                release_projected(closable)
                assert not impl.is_closed
                print(text.to_string())
                impl.dispose()
                try:
                    text.to_string()
                except OSError as error:
                    if error.winerror != -2147483629:  # RO_E_CLOSED
                        raise
                    diagnostic = impl.take_error()
                    assert isinstance(diagnostic, str) and "0x80000013" in diagnostic
                    print(diagnostic)
                else:
                    raise AssertionError("A disposed implementation accepted a native callback")
            finally:
                for view in (closable, duplicate_text, text):
                    release_projected(view)
        assert impl.is_closed


if __name__ == "__main__":
    main()
