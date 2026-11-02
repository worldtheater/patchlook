//! System application associations and file targets for the Open With menu.
use crate::git::{Change, Commit, Repository};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Clone, Debug)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub icon: Option<ApplicationIcon>,
}

#[derive(Clone, Debug)]
pub enum ApplicationIcon {
    Png(Vec<u8>),
    File(PathBuf),
}

#[derive(Clone)]
pub enum OpenWith {
    Default,
    Recommended(String),
    Program(PathBuf),
}

/// Captured when the menu opens, so later selection changes cannot retarget its actions.
#[derive(Clone)]
pub struct FileToOpen {
    pub repository: Repository,
    pub change: Change,
    pub commit: Option<Commit>,
}

impl FileToOpen {
    pub fn path(&self) -> PathBuf {
        let path = if self.change.status == 'D' {
            self.change
                .previous_path
                .as_ref()
                .unwrap_or(&self.change.path)
        } else {
            &self.change.path
        };
        self.repository.root.join(path)
    }

    fn prepare(&self) -> Result<(PathBuf, Option<TempDir>)> {
        ensure!(
            !self.repository.is_remote(),
            "External opening is only available for local repositories"
        );
        let path = self.path();
        if self.change.status != 'D' {
            ensure!(
                path.is_file(),
                "File is not present in the working tree: {}",
                path.display()
            );
            return Ok((path, None));
        }
        let bytes = self
            .repository
            .file_versions(&self.change, self.commit.as_ref())?
            .original
            .context("The version before deletion is not available")?;
        let directory = tempfile::Builder::new()
            .prefix("patchlook-open-")
            .tempdir()?;
        let copy = directory
            .path()
            .join(path.file_name().context("File has no name")?);
        std::fs::write(&copy, bytes)?;
        Ok((copy, Some(directory)))
    }

    /// The caller retains temporary copies until the application exits.
    pub fn launch(&self, application: &OpenWith) -> Result<Option<TempDir>> {
        let (path, temporary) = self.prepare()?;
        platform::open(&path, application)?;
        Ok(temporary)
    }
}

pub fn applications_for(path: &Path) -> Result<Vec<Application>> {
    let mut applications = platform::applications_for(path)?;
    applications.sort_by_cached_key(|app| (app.name.to_lowercase(), app.id.clone()));
    applications.dedup_by(|a, b| a.id == b.id);
    Ok(applications)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Area;

    fn git(path: &Path, args: &[&str]) {
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn existing_files_open_the_working_copy_even_when_viewing_history_or_index() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes with spaces.txt");
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::write(&path, "HEAD\n").unwrap();
        git(directory.path(), &["add", "."]);
        git(directory.path(), &["commit", "-qm", "Initial"]);
        let repository = Repository::open(directory.path()).unwrap();
        let commit = repository.history(0, None).unwrap().commits.remove(0);
        let historical = FileToOpen {
            change: repository.commit_changes(&commit).unwrap().remove(0),
            repository: repository.clone(),
            commit: Some(commit),
        };
        std::fs::write(&path, "INDEX\n").unwrap();
        git(directory.path(), &["add", "."]);
        std::fs::write(&path, "LATEST\n").unwrap();
        let staged = FileToOpen {
            change: repository
                .snapshot()
                .unwrap()
                .changes
                .into_iter()
                .find(|change| change.area == Area::Staged)
                .unwrap(),
            repository,
            commit: None,
        };
        for target in [&historical, &staged] {
            let (opened, temporary) = target.prepare().unwrap();
            assert_eq!(opened, target.repository.root.join("notes with spaces.txt"));
            assert!(temporary.is_none());
            assert_eq!(std::fs::read(&opened).unwrap(), b"LATEST\n");
        }
        std::fs::remove_file(path).unwrap();
        assert!(historical.prepare().is_err());
        assert!(staged.prepare().is_err());
    }

    #[test]
    fn deleted_files_use_the_matching_prior_version_without_restoring_the_worktree() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("removed.txt");
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::write(&path, "HEAD\n").unwrap();
        git(directory.path(), &["add", "."]);
        git(directory.path(), &["commit", "-qm", "Initial"]);
        std::fs::write(&path, "INDEX\n").unwrap();
        git(directory.path(), &["add", "."]);
        std::fs::remove_file(&path).unwrap();
        let repository = Repository::open(directory.path()).unwrap();
        for (area, expected) in [
            (Area::Unstaged, b"INDEX\n".as_slice()),
            (Area::Staged, b"HEAD\n".as_slice()),
        ] {
            if area == Area::Staged {
                git(directory.path(), &["add", "-u"]);
            }
            let target = FileToOpen {
                change: repository
                    .snapshot()
                    .unwrap()
                    .changes
                    .into_iter()
                    .find(|change| change.area == area && change.status == 'D')
                    .unwrap(),
                repository: repository.clone(),
                commit: None,
            };
            let (opened, temporary) = target.prepare().unwrap();
            assert_eq!(opened.file_name(), path.file_name());
            assert_eq!(std::fs::read(&opened).unwrap(), expected);
            assert!(!path.exists());
            drop(temporary);
            assert!(!opened.exists());
        }
        git(directory.path(), &["commit", "-qm", "Delete"]);
        let commit = repository.history(0, None).unwrap().commits.remove(0);
        let target = FileToOpen {
            change: repository.commit_changes(&commit).unwrap().remove(0),
            repository,
            commit: Some(commit),
        };
        std::fs::write(&path, "RECREATED\n").unwrap();
        let (opened, _temporary) = target.prepare().unwrap();
        assert_eq!(std::fs::read(&opened).unwrap(), b"HEAD\n");
        assert_eq!(std::fs::read(path).unwrap(), b"RECREATED\n");
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSFileManager, NSPoint, NSRect, NSSize, NSString, NSURL};
    use objc2_uniform_type_identifiers::UTType;

    fn application_icon(workspace: &NSWorkspace, path: &NSString) -> Option<ApplicationIcon> {
        let image = workspace.iconForFile(path);
        // Ask for the menu-sized representation, including Retina resolution.
        let mut bounds = NSRect::new(NSPoint::new(0., 0.), NSSize::new(32., 32.));
        let image = unsafe { image.CGImageForProposedRect_context_hints(&mut bounds, None, None) }?;
        let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
        // An empty property dictionary uses the system's PNG defaults.
        let data = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }?;
        Some(ApplicationIcon::Png(data.to_vec()))
    }

    pub fn applications_for(path: &Path) -> Result<Vec<Application>> {
        let workspace = NSWorkspace::sharedWorkspace();
        let applications = if path.exists() {
            let url = NSURL::from_file_path(path).context("Invalid file URL")?;
            workspace.URLsForApplicationsToOpenURL(&url)
        } else {
            let Some(content_type) = path.extension().and_then(|extension| {
                UTType::typeWithFilenameExtension(&NSString::from_str(&extension.to_string_lossy()))
            }) else {
                return Ok(Vec::new());
            };
            workspace.URLsForApplicationsToOpenContentType(&content_type)
        };
        applications
            .iter()
            .map(|url| {
                let path = url.path().context("Application has no path")?;
                let name = NSFileManager::defaultManager()
                    .displayNameAtPath(&path)
                    .to_string();
                Ok(Application {
                    id: path.to_string(),
                    name: name.strip_suffix(".app").unwrap_or(&name).to_owned(),
                    icon: application_icon(&workspace, &path),
                })
            })
            .collect()
    }

    #[test]
    fn missing_files_still_have_system_recommended_applications() {
        let directory = tempfile::tempdir().unwrap();
        let applications = applications_for(&directory.path().join("deleted.txt")).unwrap();
        assert!(!applications.is_empty());
        assert!(applications.iter().any(|app| matches!(
            &app.icon,
            Some(ApplicationIcon::Png(bytes)) if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        )));
    }

    pub fn open(path: &Path, application: &OpenWith) -> Result<()> {
        let mut command = std::process::Command::new("/usr/bin/open");
        match application {
            OpenWith::Default => {}
            OpenWith::Recommended(id) => {
                command.arg("-a").arg(id);
            }
            OpenWith::Program(program) => {
                command.arg("-a").arg(program);
            }
        }
        let output = command.arg("--").arg(path).output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use gio::{AppInfo, AppInfoCreateFlags, AppLaunchContext, DesktopAppInfo, File, prelude::*};

    fn application_icon(app: &AppInfo, theme: Option<&str>) -> Option<ApplicationIcon> {
        let icon = app.icon()?;
        if let Some(file) = icon.downcast_ref::<gio::FileIcon>() {
            return file.file().path().map(ApplicationIcon::File);
        }
        let icon = icon.downcast_ref::<gio::ThemedIcon>()?;
        icon.names().iter().find_map(|name| {
            let mut lookup = freedesktop_icons::lookup(name).with_size(32);
            if let Some(theme) = theme {
                lookup = lookup.with_theme(theme);
            }
            lookup.find().map(ApplicationIcon::File)
        })
    }

    pub fn applications_for(path: &Path) -> Result<Vec<Application>> {
        let (content_type, _) = gio::content_type_guess(Some(path), None::<&[u8]>);
        let theme = freedesktop_icons::default_theme_gtk();
        Ok(AppInfo::recommended_for_type(&content_type)
            .into_iter()
            .filter(|app| app.should_show())
            .filter_map(|app| {
                app.id().map(|id| Application {
                    id: id.to_string(),
                    name: app.display_name().to_string(),
                    icon: application_icon(&app, theme.as_deref()),
                })
            })
            .collect())
    }

    pub fn open(path: &Path, application: &OpenWith) -> Result<()> {
        let file = File::for_path(path);
        let app: AppInfo = match application {
            OpenWith::Default => {
                return Ok(AppInfo::launch_default_for_uri(
                    &file.uri(),
                    None::<&AppLaunchContext>,
                )?);
            }
            OpenWith::Recommended(id) => DesktopAppInfo::new(id)
                .context("Application is no longer installed")?
                .upcast(),
            OpenWith::Program(program)
                if program.extension().is_some_and(|ext| ext == "desktop") =>
            {
                DesktopAppInfo::from_filename(program)
                    .context("Invalid desktop application")?
                    .upcast()
            }
            OpenWith::Program(program) => AppInfo::create_from_commandline(
                gio::glib::shell_quote(program),
                None,
                AppInfoCreateFlags::NONE,
            )?,
        };
        app.launch(&[file], None::<&AppLaunchContext>)?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use windows::{
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            Graphics::Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory,
                IWICPalette, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
            },
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IBindCtx, IDataObject,
            },
            UI::{
                Shell::{
                    ASSOC_FILTER_RECOMMENDED, BHID_DataObject, IAssocHandler, IShellItem,
                    SHAssocEnumHandlers, SHCreateItemFromParsingName, SHDefExtractIconW,
                    ShellExecuteW,
                },
                WindowsAndMessaging::{DestroyIcon, HICON, SW_SHOWNORMAL},
            },
        },
        core::{HSTRING, PCWSTR, PWSTR},
    };

    fn with_com<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
        // GPUI's UI thread may already have a COM apartment; background launches do not.
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if initialized != RPC_E_CHANGED_MODE {
            initialized.ok()?;
        }
        let result = operation();
        if initialized.is_ok() {
            unsafe { CoUninitialize() };
        }
        result
    }

    unsafe fn take_string(value: PWSTR) -> Result<String> {
        // IAssocHandler allocates these strings with the COM task allocator.
        let result = unsafe { value.to_string() };
        unsafe { CoTaskMemFree(Some(value.0.cast())) };
        Ok(result?)
    }

    fn handlers(path: &Path) -> Result<Vec<IAssocHandler>> {
        let Some(extension) = path.extension() else {
            return Ok(Vec::new());
        };
        let extension = HSTRING::from(format!(".{}", extension.to_string_lossy()));
        let enumerator = unsafe { SHAssocEnumHandlers(&extension, ASSOC_FILTER_RECOMMENDED)? };
        let mut handlers = Vec::new();
        loop {
            let mut item = [None];
            let mut fetched = 0;
            unsafe { enumerator.Next(&mut item, Some(&mut fetched))? };
            if fetched == 0 {
                break;
            }
            handlers.push(item[0].take().context("Missing application handler")?);
        }
        Ok(handlers)
    }

    fn application_icon(
        handler: &IAssocHandler,
        factory: &IWICImagingFactory,
    ) -> Result<ApplicationIcon> {
        let mut path = PWSTR::null();
        let mut index = 0;
        unsafe { handler.GetIconLocation(&mut path, &mut index)? };
        let path = unsafe { take_string(path)? };
        let mut icon = HICON::default();
        unsafe {
            SHDefExtractIconW(&HSTRING::from(path), index, 0, Some(&mut icon), None, 32).ok()?
        };
        // WIC handles both modern alpha icons and legacy icon transparency masks.
        let result = (|| -> Result<ApplicationIcon> {
            let bitmap = unsafe { factory.CreateBitmapFromHICON(icon)? };
            let converter = unsafe { factory.CreateFormatConverter()? };
            unsafe {
                converter.Initialize(
                    &bitmap,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None::<&IWICPalette>,
                    0.,
                    WICBitmapPaletteTypeCustom,
                )?;
            }
            let (mut width, mut height) = (0, 0);
            unsafe { converter.GetSize(&mut width, &mut height)? };
            let mut pixels = vec![0; (width * height * 4) as usize];
            unsafe { converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)? };
            let mut png = Vec::new();
            let mut encoder = png::Encoder::new(&mut png, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&pixels)?;
            Ok(ApplicationIcon::Png(png))
        })();
        unsafe {
            let _ = DestroyIcon(icon);
        }
        result
    }

    pub fn applications_for(path: &Path) -> Result<Vec<Application>> {
        with_com(|| {
            let factory: Option<IWICImagingFactory> = unsafe {
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
            };
            handlers(path)?
                .iter()
                .map(|handler| unsafe {
                    Ok(Application {
                        id: take_string(handler.GetName()?)?,
                        name: take_string(handler.GetUIName()?)?,
                        icon: factory
                            .as_ref()
                            .and_then(|factory| application_icon(handler, factory).ok()),
                    })
                })
                .collect()
        })
    }

    pub fn open(path: &Path, application: &OpenWith) -> Result<()> {
        with_com(|| match application {
            OpenWith::Default => {
                let result = unsafe {
                    ShellExecuteW(
                        None,
                        PCWSTR::null(),
                        &HSTRING::from(path.as_os_str()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    )
                };
                ensure!(
                    result.0 as isize > 32,
                    "Windows could not open the file (code {})",
                    result.0 as isize
                );
                Ok(())
            }
            OpenWith::Program(program) => {
                let mut child = std::process::Command::new(program).arg(path).spawn()?;
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Ok(())
            }
            OpenWith::Recommended(id) => {
                for handler in handlers(path)? {
                    if unsafe { take_string(handler.GetName()?)? } == *id {
                        let item: IShellItem = unsafe {
                            SHCreateItemFromParsingName(
                                &HSTRING::from(path.as_os_str()),
                                None::<&IBindCtx>,
                            )?
                        };
                        let data: IDataObject =
                            unsafe { item.BindToHandler(None::<&IBindCtx>, &BHID_DataObject)? };
                        unsafe { handler.Invoke(&data)? };
                        return Ok(());
                    }
                }
                anyhow::bail!("Application is no longer registered for this file type")
            }
        })
    }
}
