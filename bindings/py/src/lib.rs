// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use pyo3::prelude::*;

mod async_runtime;
mod runtime;
mod values;

#[pymodule]
mod dynwinrt_py {
    use pyo3::prelude::*;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        super::async_runtime::init_async_runtime();
        m.py().run(
            c"
from typing import Awaitable as _Awaitable, Callable as _Callable
from typing import Protocol as _Protocol, TypeVar as _TypeVar

_T = _TypeVar('_T', covariant=True)
_P = _TypeVar('_P', covariant=True)

class WinRTAsync(_Awaitable[_T], _Protocol[_T]):
    def wait(self) -> _T: ...
    def cancel(self) -> None: ...

class WinRTAsyncWithProgress(WinRTAsync[_T], _Protocol[_T, _P]):
    def progress(self, callback: _Callable[[_P], object]) -> None: ...

async def _dynwinrt_convert_future(future, converter):
    try:
        completed = await future
        return converter(completed._get_async_results())
    except BaseException:
        if not future.done():
            future.cancel()
        raise

def _dynwinrt_link_cancellation(task, future):
    def cancel_inner(completed):
        if completed.cancelled() and not future.done():
            future.cancel()
    task.add_done_callback(cancel_inner)

def _dynwinrt_dispatch_progress(callback, converter, value):
    callback(converter(value))
",
            Some(&m.dict()),
            None,
        )?;

        // Classes
        m.add_class::<super::runtime::WinAppSDKContext>()?;
        m.add_class::<super::runtime::WinGUID>()?;
        m.add_class::<super::runtime::DynWinRTType>()?;
        m.add_class::<super::runtime::DynWinRTMethodSig>()?;
        m.add_class::<super::runtime::DynWinRTMethodHandle>()?;
        m.add_class::<super::runtime::DynWinRTValue>()?;
        m.add_class::<super::runtime::DynWinRTArray>()?;
        m.add_class::<super::runtime::DynWinRTStruct>()?;
        m.add_class::<super::runtime::DynWinRtDelegate>()?;
        m.add_class::<super::async_runtime::DynWinRTAsync>()?;
        m.add_class::<super::async_runtime::DynWinRTAsyncWithProgress>()?;

        // Functions
        m.add_function(wrap_pyfunction!(super::runtime::init_winappsdk, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::ro_initialize, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::ro_uninitialize, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::has_package_identity, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::get_computer_name, m)?)?;

        m.py().run(
            c"
__all__ = [name for name in __all__ if not name.startswith('_DynWinRTAsync')]
for _name in ('WinRTAsync', 'WinRTAsyncWithProgress'):
    if _name not in __all__:
        __all__.append(_name)
",
            Some(&m.dict()),
            None,
        )?;

        Ok(())
    }
}
