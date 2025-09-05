use std::{mem, sync::Arc};

#[cfg(target_arch = "wasm32")]
use winit::platform::web::EventLoopExtWebSys;
use winit::{
    event::WindowEvent,
    event_loop::{ControlFlow, EventLoopProxy},
};

use crate::{
    client::render,
    common::runtime::{self, HandleInterface, TaskHandle, TaskHandleInterface},
};

#[derive(Debug)]
enum Event {
    Resumed,
    Close,
    ContextReady,
    Draw,
}

enum State {
    Working,
    New,
    Starting(Arc<winit::window::Window>, TaskHandle<render::Context>),
    Active(Arc<winit::window::Window>, render::Context),
    Terminated,
}

struct Application {
    rt_handle: runtime::Handle,
    state: State,
    event_proxy: EventLoopProxy<Event>,
}

impl Application {
    fn new(rt_handle: runtime::Handle, event_proxy: EventLoopProxy<Event>) -> Self {
        Self {
            rt_handle,
            state: State::New,
            event_proxy,
        }
    }

    fn process_event(&mut self, event: Event, event_loop: &winit::event_loop::ActiveEventLoop) {
        let state = mem::replace(&mut self.state, State::Working);

        let next_state = match (state, event) {
            (State::New, Event::Resumed) => {
                let window = Arc::new(
                    event_loop
                        .create_window(winit::window::WindowAttributes::default())
                        .unwrap(),
                );

                let proxy = self.event_proxy.clone();
                let ctx_fut = render::Context::new(window.clone());

                State::Starting(
                    window,
                    self.rt_handle.launch_task_async(async move |_| {
                        let ctx = ctx_fut.await;
                        proxy.send_event(Event::ContextReady).unwrap();
                        ctx
                    }),
                )
            }
            (State::Starting(window, handle), Event::ContextReady) => {
                let ctx = handle.join();
                let size = window.inner_size();
                ctx.resize(size.width, size.height);
                window.request_redraw();
                State::Active(window, ctx)
            }
            (State::Active(window, ctx), Event::Draw) => {
                window.request_redraw();
                ctx.draw();
                State::Active(window, ctx)
            }
            (_, Event::Close) => {
                self.rt_handle.stop();
                event_loop.exit();
                State::Terminated
            }
            (state, _) => state,
        };
        self.state = next_state;
    }
}

impl winit::application::ApplicationHandler<Event> for Application {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.process_event(Event::Resumed, event_loop);
    }

    fn exiting(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.process_event(Event::Close, event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        window_event: WindowEvent,
    ) {
        let event = match window_event {
            WindowEvent::CloseRequested => Some(Event::Close),
            WindowEvent::RedrawRequested => Some(Event::Draw),
            _ => None,
        };

        if let Some(event) = event {
            self.process_event(event, event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: Event) {
        self.process_event(event, event_loop);
    }
}

pub fn start(handle: runtime::Handle) {
    let event_loop = winit::event_loop::EventLoop::<Event>::with_user_event()
        .build()
        .unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let event_proxy = event_loop.create_proxy();

    let stopper_proxy = event_proxy.clone();
    handle.launch_task_async(async move |handle| {
        handle.stopped().await;
        stopper_proxy.send_event(Event::Close).ok();
    });

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = Application::new(handle, event_proxy);
        event_loop.run_app(&mut app).unwrap();
    }

    #[cfg(target_arch = "wasm32")]
    event_loop.spawn_app(Application::new(handle, event_proxy));
}
