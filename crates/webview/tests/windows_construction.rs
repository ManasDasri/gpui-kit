//! Real WebView2 regression for #3984. Run explicitly on Windows:
//! cargo test -p gpui-webview --features gpui-fast,gpui-kit/gpui-fast --test windows_construction
//! Add `-- --expect-crash` to verify the original failure in isolated processes.

#[cfg(target_os = "windows")]
mod native {
    use gpui_kit::{
        App, AppContext as _, AsyncApp, Context, Entity, IntoElement, ParentElement as _, QuitMode,
        Render, Window, WindowOptions, div,
    };
    use gpui_webview::WebView;
    use std::{
        cell::{Cell, RefCell},
        fs::File,
        process::{Command, Stdio},
        rc::Rc,
        thread,
        time::{Duration, Instant},
    };
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{KillTimer, SetTimer},
    };

    struct View(Entity<WebView>);

    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(self.0.clone())
        }
    }

    fn build(cx: &mut App) {
        let building = Rc::new(Cell::new(true));
        let updates = cx.new(|_| 0usize);
        cx.spawn({
            let building = building.clone();
            let updates = updates.clone();
            async move |cx| {
                eprintln!("foreground task polled; building={}", building.get());
                cx.update(|_| ());
                updates.update(cx, |count, _| *count += 1);
                assert!(
                    !building.get(),
                    "foreground task entered borrowed construction"
                );
                eprintln!("queued app and entity updates completed");
            }
        })
        .detach();

        eprintln!("starting native WebView construction");
        let (window, view) = gpui_kit::open_window(
            WindowOptions {
                show: false,
                focus: false,
                ..Default::default()
            },
            cx,
            |window, cx| {
                let webview = cx.new(|cx| {
                    WebView::build(
                        wry::WebViewBuilder::new()
                            .with_html("<!doctype html><title>GPUI regression</title>")
                            .with_initialization_script("window.regression = true;"),
                        window,
                        cx,
                    )
                    .expect("create real WebView2, including IPC and initialization scripts")
                });
                cx.new(|_| View(webview))
            },
        )
        .expect("open native GPUI window");
        building.set(false);
        eprintln!("native WebView construction returned");

        cx.spawn(async move |cx| {
            // The queued update must resume even if no further input arrives.
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            cx.update(|cx| assert_eq!(*updates.read(cx), 1, "lost foreground wakeup"));
            window
                .update(cx, |_, window, _| window.remove_window())
                .unwrap();
            drop(view);
            eprintln!("PASS: WebView built, queued updates resumed, window closed");
            cx.update(|cx| cx.quit());
        })
        .detach();
    }

    thread_local! {
        static LATER: RefCell<Option<AsyncApp>> = const { RefCell::new(None) };
    }

    unsafe extern "system" fn later(_: HWND, _: u32, timer: usize, _: u32) {
        // A timer callback, like a click handler, starts outside a foreground task.
        unsafe { KillTimer(None, timer).unwrap() };
        let cx = LATER.with(|slot| slot.borrow_mut().take().unwrap());
        cx.update(build);
    }

    fn child(case: &str) {
        unsafe { std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true") };
        let startup = case == "startup";
        gpui_kit::application()
            .with_quit_mode(QuitMode::Explicit)
            .run(move |cx| {
                gpui_kit::init(cx);
                if startup {
                    build(cx);
                } else {
                    LATER.with(|slot| *slot.borrow_mut() = Some(cx.to_async()));
                    assert_ne!(unsafe { SetTimer(None, 0, 10, Some(later)) }, 0);
                }
            });
        eprintln!("application returned normally");
    }

    pub fn main() {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("--case") {
            child(&args[1]);
            return;
        }
        let expect_crash = args.iter().any(|arg| arg == "--expect-crash");
        for case in ["startup", "later"] {
            let log = std::env::temp_dir()
                .join(format!("gpui-webview-{}-{case}.log", std::process::id()));
            let file = File::create(&log).unwrap();
            let mut process = Command::new(std::env::current_exe().unwrap())
                .args(["--case", case])
                .env("RUST_BACKTRACE", "1")
                .stdout(Stdio::from(file.try_clone().unwrap()))
                .stderr(Stdio::from(file))
                .spawn()
                .unwrap();
            let started = Instant::now();
            let status = loop {
                if let Some(status) = process.try_wait().unwrap() {
                    break status;
                }
                if started.elapsed() > Duration::from_secs(60) {
                    process.kill().unwrap();
                    process.wait().unwrap();
                    panic!(
                        "{case} timed out; log: {}",
                        std::fs::read_to_string(&log).unwrap()
                    );
                }
                thread::sleep(Duration::from_millis(20));
            };
            let output = std::fs::read_to_string(&log).unwrap();
            println!("{case}: {status}\n{output}");
            if expect_crash {
                assert!(!status.success(), "baseline did not crash");
                assert!(
                    output.contains("RefCell already borrowed"),
                    "unrelated failure"
                );
                assert!(output.contains("starting native WebView construction"));
            } else {
                assert!(status.success(), "{case} failed");
                assert!(output.contains("PASS: WebView built"));
                assert!(output.contains("application returned normally"));
                assert!(!output.contains("RefCell already borrowed"));
            }
        }
    }
}

fn main() {
    #[cfg(target_os = "windows")]
    native::main();
    #[cfg(not(target_os = "windows"))]
    eprintln!("windows_construction requires Windows and WebView2");
}
