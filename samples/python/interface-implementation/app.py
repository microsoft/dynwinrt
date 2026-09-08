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
        instance_owner = IBackgroundTaskInstance.implement(TaskInstance())
        handlers = Task()
        owner = IBackgroundTask.implement(
            handlers,
            IStringable.implementation(handlers),
            IClosable.implementation(handlers),
        )
        instance = IBackgroundTaskInstance.from_implementation(instance_owner)
        task = IBackgroundTask.from_implementation(owner)
        text = IStringable.from_implementation(owner)
        duplicate_text = IStringable.from_implementation(owner)
        closable = IClosable.from_implementation(owner)
        try:
            assert text is not duplicate_text
            release_projected(duplicate_text)
            # Run and its nested progress property calls traverse native vtables.
            task.run(instance)
            assert instance.progress == 1
            print(text.to_string())
            owner.release()
            task.run(instance)
            assert instance.progress == 2
            closable.close()
            assert handlers.closes == 1
            release_projected(closable)
            assert not owner.is_closed
            print(text.to_string())
            owner.dispose()
            try:
                text.to_string()
            except OSError as error:
                if error.winerror != -2147483629:  # RO_E_CLOSED
                    raise
                diagnostic = owner.take_error()
                assert isinstance(diagnostic, str) and "0x80000013" in diagnostic
                print(diagnostic)
            else:
                raise AssertionError("A disposed implementation accepted a native callback")
        finally:
            owner.dispose()
            instance_owner.dispose()
            for view in (closable, duplicate_text, text, task, instance):
                release_projected(view)
        assert owner.is_closed


if __name__ == "__main__":
    main()
