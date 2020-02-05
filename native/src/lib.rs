use neon::prelude::*;
use neon::context::Context;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::runtime;
use wickdl::{ServiceState, EncryptedPak, PakService};

pub struct RuntimeContainerInner {
    runtime: runtime::Runtime,
    // The mutex is only there to set the Option. Please kill me.
    service: Mutex<Option<Arc<ServiceState>>>,
    next_counter: AtomicU32,
}

pub struct RuntimeContainer {
    inner: Arc<RuntimeContainerInner>,
    cb: EventHandler,
}

pub struct EncryptedPakContainer {
    pak: Option<EncryptedPak>,
}

pub struct PakContainer {
    pak: Option<PakService>,
}

declare_types! {
    pub class JsRuntimeContainer for RuntimeContainer {
        init(mut cx) {
            let rt = runtime::Builder::new()
                .enable_all()
                .threaded_scheduler()
                .core_threads(4)
                .build()
                .unwrap();

            let js_cb = cx.argument::<JsFunction>(0)?;
            let this = cx.this();
            let cb = EventHandler::new(&cx, this, js_cb);

            Ok(RuntimeContainer {
                inner: Arc::new(RuntimeContainerInner {
                    runtime: rt,
                    service: Mutex::new(None),
                    next_counter: AtomicU32::new(1),
                }),
                cb,
            })
        }

        method start_service(mut cx) {
            let this = cx.this();
            let (cb, state) = {
                let guard = cx.lock();
                let data = this.borrow(&guard);
                (data.cb.clone(), Arc::clone(&data.inner))
            };
            let counter = state.next_counter.fetch_add(1, Ordering::AcqRel) as f64;

            // Still don't know how to do this properly
            let state2 = Arc::clone(&state);

            state.runtime.spawn(async move {
                let args: Vec<f64> = match ServiceState::new().await {
                    Ok(service) => {
                        let mut lock = state2.service.lock().unwrap();
                        *lock = Some(Arc::new(service));
                        vec![counter, 0.0]
                    },
                    Err(_) => {
                        vec![counter, 1.0]
                    },
                };
                cb.schedule(move |cx| {
                    let res: Vec<Handle<JsValue>> = args.iter().map(|&v| cx.number(v).upcast()).collect();
                    res
                });
            });

            Ok(cx.number(counter).upcast())
        }

        method get_paks(mut cx) {
            let this = cx.this();
            let state = {
                let guard = cx.lock();
                let data = this.borrow(&guard);
                Arc::clone(&data.inner)
            };

            let service = Arc::clone(state.service.lock().unwrap().as_ref().unwrap());
            let names = service.get_paks();

            let js_array = JsArray::new(&mut cx, names.len() as u32);
            for (i, item) in names.iter().enumerate() {
                let js_string = cx.string(item);
                js_array.set(&mut cx, i as u32, js_string)?;
            }

            Ok(js_array.upcast())
        }

        method get_pak(mut cx) {
            let this = cx.this();
            let pak_name = cx.argument::<JsString>(0)?.value();
            let (cb, state) = {
                let guard = cx.lock();
                let data = this.borrow(&guard);
                (data.cb.clone(), Arc::clone(&data.inner))
            };

            let service = Arc::clone(state.service.lock().unwrap().as_ref().unwrap());
            let counter = state.next_counter.fetch_add(1, Ordering::AcqRel) as f64;

            state.runtime.spawn(async move {
                match service.get_pak(pak_name).await {
                    Ok(pak) => {
                        cb.schedule(move |tcx| -> Vec<Handle<JsValue>> {
                            let mut pak_container = JsEncryptedPak::new::<_, JsEncryptedPak, _>(tcx, vec![]).unwrap();
                            let guard = tcx.lock();
                            pak_container.borrow_mut(&guard).pak = Some(pak);
                            vec![tcx.number(counter).upcast(), tcx.number(0.0).upcast(), pak_container.upcast()]
                        });
                    },
                    Err(_) => {
                        cb.schedule(move |tcx| -> Vec<Handle<JsValue>> {
                            vec![tcx.number(counter).upcast(), tcx.number(1.0).upcast()]
                        });
                    },
                }
            });

            Ok(cx.number(counter).upcast())
        }

        method decrypt_pak(mut cx) {
            let this = cx.this();
            let mut pak = cx.argument::<JsEncryptedPak>(0)?;
            let key = cx.argument::<JsString>(1)?.value();
            let (cb, state) = {
                let guard = cx.lock();
                let data = this.borrow(&guard);
                (data.cb.clone(), Arc::clone(&data.inner))
            };
            let encpakop = {
                let guard = cx.lock();
                let mut data = pak.borrow_mut(&guard);
                data.pak.take()
            };
            let encpak = match encpakop {
                Some(inner) => inner,
                None => return cx.throw_error("Pak already consumed"),
            };

            let service = Arc::clone(state.service.lock().unwrap().as_ref().unwrap());
            let counter = state.next_counter.fetch_add(1, Ordering::AcqRel) as f64;

            state.runtime.spawn(async move {
                match service.decrypt_pak(encpak, key).await {
                    Ok(pak) => {
                        cb.schedule(move |tcx| -> Vec<Handle<JsValue>> {
                            let mut pak_container = JsPakContainer::new::<_, JsPakContainer, _>(tcx, vec![]).unwrap();
                            let guard = tcx.lock();
                            pak_container.borrow_mut(&guard).pak = Some(pak);
                            vec![tcx.number(counter).upcast(), tcx.number(0.0).upcast(), pak_container.upcast()]
                        })
                    },
                    Err(_) => {
                        cb.schedule(move |tcx| -> Vec<Handle<JsValue>> {
                            vec![tcx.number(counter).upcast(), tcx.number(1.0).upcast()]
                        })
                    },
                }
            });

            Ok(cx.number(counter).upcast())
        }
    }

    pub class JsEncryptedPak for EncryptedPakContainer {
        init(_) {
            Ok(EncryptedPakContainer {
                pak: None,
            })
        }
    }

    pub class JsPakContainer for PakContainer {
        init(_) {
            Ok(PakContainer {
                pak: None,
            })
        }

        method get_pak_mount(mut cx) {
            let this = cx.this();
            let contents = {
                let guard = cx.lock();
                let data = this.borrow(&guard);
                match data.pak {
                    Some(inner) => Some(inner.get_mount_point().to_owned()),
                    None => None,
                }
            };
            let mount = match contents {
                Some(inner) => inner,
                None => return cx.throw_error("No pak inside container"),
            };

            Ok(JsString::new(&mut cx, mount).upcast())
        }

        method get_file_names(mut cx) {

        }
    }
}

register_module!(mut cx, {
    cx.export_class::<JsRuntimeContainer>("RuntimeContainer")?;

    Ok(())
});