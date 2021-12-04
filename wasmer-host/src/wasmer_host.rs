use wasmer::ImportObject;
use wasmer::{Module, Store, Instance, Val};
use wasmer_wasi::WasiState;
use anyhow::{Result};
use protocol::{WasmHost, RunModuleFunctionParameters};
use runtime_registry::registry::RuntimeRegistry;
use wasmer::Value;
use wasmer_compiler_cranelift::Cranelift;
use wasmer_engine_universal::Universal;
use wasmer_wasi_experimental_network::runtime_impl::get_namespace;

pub struct WasmerHost {
    instance: Instance,
    /// Pointer to javascript runtime
    js_rt_ptr: i32,
    /// Pointer to async runtime
    async_rt_ptr: i32,
    /// The internal wasm buffer offset
    parameter_buffer_ptr: i32,
}

impl WasmerHost {
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    pub fn new_wasi_dev(runtime: &str) -> Self {
        println!("Creating wasi dev instance.");
        let instance = WasmerHost::new_wasi_dev_instance(runtime);
        println!("Wasi dev instance created.");
        // Init js runtime object for reuse
        let new_js_rt_ptr = instance.exports.get_function("new_runtime").unwrap();
        let js_rt_ptr = new_js_rt_ptr.call(&[]).unwrap()[0].i32().unwrap();
        // Init async runtime for reuse
        let new_async_rt_ptr = instance.exports.get_function("new_async_runtime").unwrap();
        let async_rt_ptr = new_async_rt_ptr.call(&[]).unwrap()[0].i32().unwrap();
        // Get the buffer pointer
        let buffer_fn = instance.exports.get_function("parameter_buffer_ptr").unwrap();
        let parameter_buffer_ptr = buffer_fn.call(&[]).unwrap()[0].i32().unwrap();
        WasmerHost {
            instance,
            js_rt_ptr,
            async_rt_ptr,
            parameter_buffer_ptr,
        }
    }

    /// Instantiates a wasmer instance and initializes it with wasi host functions
    /// and experimental network host functions for development environment.
    fn new_wasi_dev_instance(runtime: &str) -> Instance {
        let registry = RuntimeRegistry::new();
        let store = Store::new(&Universal::new(Cranelift::default()).engine());
        // Check cache for module
        let module = registry.get_module(runtime, &store).unwrap();
        let import_object = init_wasi_dev_imports(&store, &module);
        let instance = Instance::new(&module, &import_object).unwrap();
        instance
    }

    #[inline]
    fn read_returned_value(&self, buffer: &mut [u8], len: i32) -> Vec<u8> {
        buffer[self.parameter_buffer_ptr as usize..(self.parameter_buffer_ptr as usize + len as usize)]
            .to_vec()
    }

    #[inline]
    fn slice_to_buffer(&self, buffer: &mut [u8], source: &[u8]) {
        buffer[self.parameter_buffer_ptr as usize..(self.parameter_buffer_ptr as usize + source.len())].copy_from_slice(source);
    }
}

impl WasmHost for WasmerHost {

    fn compile_to_bytecode(&mut self, _: &str, code: &str) -> Result<std::vec::Vec<u8>> {
        let source = code.as_bytes();
        let memory = self.instance.exports.get_memory("memory")?;
        let data = unsafe { memory.data_unchecked_mut() };
        // Copy source code to the buffer
        self.slice_to_buffer(data, source);
        // Get function pointer
        let compile_module_fn = self.instance.exports.get_function("compile_module")?;
        // Cal the function
        println!("Calling compile_module_fn...");
        let bytecode_size = compile_module_fn.call(&[
            Value::I32(self.async_rt_ptr as i32),
            Value::I32(self.js_rt_ptr as i32),
            Value::I32(source.len() as i32),
        ])?[0]
            .i32()
            .unwrap();
        // Copy returned data
        let mut bytecode = Vec::with_capacity(bytecode_size as usize);
        bytecode.resize(bytecode_size as usize, 0);
        bytecode.copy_from_slice(
            &data[self.parameter_buffer_ptr as usize..(self.parameter_buffer_ptr as usize + bytecode_size as usize)],
        );
        println!("Done compiling to bytecode.");
        Ok(bytecode)
    }

    fn eval(&self, js: &str) {
        let js_bytes = js.as_bytes();
        let memory = self.instance.exports.get_memory("memory").unwrap();
        let js_rt_eval_fn = self
            .instance
            .exports
            .get_function("run")
            .unwrap();
        let data = unsafe { memory.data_unchecked_mut() };
        self.slice_to_buffer(data, js_bytes);
        js_rt_eval_fn.call(&[
            Val::I32(self.async_rt_ptr)
            Val::I32(self.js_rt_ptr),
            Val::I32(js_bytes.len() as i32)]).unwrap()[0]
            .i32()
            .unwrap();
    }

    fn run_module_function(&self, parameters: &mut RunModuleFunctionParameters) -> Result<String> {
        let memory = self.instance.exports.get_memory("memory").unwrap();
        let run_module_function = self
            .instance
            .exports
            .get_function("run_module_function")
            .unwrap();
        // Get the wasm memory as mutable slice.
        let data = unsafe { memory.data_unchecked_mut() };
        parameters.set_rt(self.js_rt_ptr as u32);
        let serialized = bincode::serialize(&parameters).unwrap();
        self.slice_to_buffer(data, &serialized);
        let res = run_module_function.call(&[
            Val::I32(self.async_rt_ptr)
            Val::I32(serialized.len() as i32)])?[0]
            .i32()
            .unwrap();
        let json_bytes = self.read_returned_value(data, res);
        Ok(String::from_utf8(json_bytes).unwrap())
    }
}

pub fn init_wasi_dev_imports(store: &Store, module: &Module) -> ImportObject {
    let mut wasi_env = WasiState::new("state")
        .finalize().unwrap();
    let mut import_object = wasi_env.import_object(&module).unwrap();
    let (n, exports) = get_namespace(store, &wasi_env);
    import_object.register(n, exports);
    import_object
}

pub fn compile_to_bytecode(runtime: &str, js: &str) -> Result<Vec<u8>> {
    let mut rt = WasmerHost::new_wasi_dev(runtime);
    rt.compile_to_bytecode("mod1", js)
}
