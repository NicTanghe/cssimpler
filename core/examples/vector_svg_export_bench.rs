//! Compare export peak RSS in fresh processes. Pass `buffered|string` and a file path.
use cssimpler_core::{VectorPathBuilder, VectorPoint as P, VectorScene, VectorViewBox};
use std::{
    env,
    fs::{self, File},
    io::{self, BufWriter, Write},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let mode = args.next().and_then(|arg| arg.into_string().ok());
    let destination = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected buffered|string and a file path",
        )
    })?;
    let mut path = VectorPathBuilder::new().move_to(P::new(0.25, 0.5));
    for i in 0..16 {
        let x = (i * 5) as f32;
        path = path.cubic_to(
            P::new(x + 1., 10.),
            P::new(x + 2., 20.),
            P::new(x + 5., 2.5),
        );
    }
    let path = path.close().build()?;
    let scene = VectorScene::new(VectorViewBox::new(0., 0., 100., 100.), vec![path; 200_000])?;
    match mode.as_deref() {
        Some("buffered") => {
            let mut file = BufWriter::new(File::create(&destination)?);
            scene.write_svg(&mut file)?;
            file.flush()?;
        }
        Some("string") => fs::write(&destination, scene.to_svg())?,
        _ => {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "expected buffered or string").into(),
            );
        }
    }
    let peak = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let peak = peak
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .map(str::trim)
        .unwrap_or("unavailable");
    println!(
        "{} bytes, peak RSS {peak}",
        fs::metadata(&destination)?.len()
    );
    Ok(())
}
