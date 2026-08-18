# mfm-app

Portfolio-only thin facade over one already-composed Runtime. `Application::new` accepts Runtime
alone. `start_portfolio` plans checked Program/C0 from selector, process-local config, and targets,
then calls Runtime with the caller's explicit RunId. `resume` and `read` return Runtime RunView
directly.

`start_portfolio` rejects more than 64 target descriptors before cloning, planning, or Runtime
entry. It moves only bounded, secret-free planning inputs into an immediately awaited blocking
task; Runtime execution remains asynchronous.

Invalid selector is `InvalidRequest`; trusted planner/composition failure is `Internal`; typed
Runtime failures pass through `ApplicationError::Runtime`. Application owns no session, frame fold,
status projection, provider handle, configuration service, or transaction submission.
