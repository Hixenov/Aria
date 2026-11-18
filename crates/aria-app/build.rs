//! Builds Aria's icon into `aria.exe` on Windows. Elsewhere this does nothing: the window gets its
//! icon at run time (see `aria_app::icon`).

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=windows/aria.rc");
    println!("cargo:rerun-if-changed=../../assets/icon/aria.ico");
    // On Windows a missing resource compiler fails the build, so a release is never made without
    // its icon by mistake.
    embed_resource::compile("windows/aria.rc", embed_resource::NONE).manifest_required()?;
    Ok(())
}
