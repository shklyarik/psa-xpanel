use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::time::{SystemTime, UNIX_EPOCH};
use chrono::Local;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::protocol::Event;
use x11rb::CURRENT_TIME;

use ab_glyph::{FontRef, Font, PxScale, ScaleFont, point};
use image::{RgbaImage, ImageBuffer, imageops::FilterType};

// ... (КОНСТАНТЫ ОСТАЮТСЯ БЕЗ ИЗМЕНЕНИЙ) ...
const PANEL_HEIGHT: u16 = 38;
const ICON_SIZE: u16 = 24;
const BG_COLOR: u32 = 0x1d1f21;
const ACTIVE_BG_COLOR: u32 = 0x373b41;
const HOVER_BG_COLOR: u32 = 0x282a2e;
const TEXT_COLOR: u32 = 0xe0e0e0;
const DATE_COLOR: u32 = 0x969896;
const TRAY_ICON_WIDTH: u16 = 32;
const UNDERLINE_COLOR: u32 = 0x5FAFAF;
const UNDERLINE_HEIGHT: u16 = 2;
const FONT_PATH: &str = "/usr/share/fonts/TTF/OpenSans-Light.ttf";
const FONT_SIZE_MAIN: f32 = 15.0;
const FONT_SIZE_DATE: f32 = 12.0;
const TEXT_Y_OFFSET: i16 = 11;
const ICON_Y_OFFSET: i16 = 6;

// ... (STRUCT CachedWindowData ОСТАЕТСЯ БЕЗ ИЗМЕНЕНИЙ) ...
struct CachedWindowData {
    title: String,
    icon_buffer: Option<Vec<u8>>,
    icon_width: u16,
    icon_height: u16,
}

// В AppState добавим буфер для списка окон, чтобы не аллоцировать его каждый кадр (Оптимизация)
struct AppState<'a> {
    conn: RustConnection,
    atoms: Atoms,
    screen_num: usize,
    win_id: Window,
    pixmap_id: Pixmap,
    gc_id: Gcontext,
    width: u16,
    tray_icons: Vec<Window>,
    click_regions: Vec<(i16, i16, Window)>,
    font: FontRef<'a>,
    mouse_x: i16,
    window_cache: HashMap<Window, CachedWindowData>,
    hovered_window: Option<Window>,
    render_buffer: Vec<u8>,
    raw_windows_buf: Vec<Window>, // <--- !!! Добавлено для оптимизации
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ... (ЗАГРУЗКА ШРИФТОВ БЕЗ ИЗМЕНЕНИЙ) ...
    let font_path = if std::path::Path::new("/usr/share/fonts/TTF/DejaVuSans.ttf").exists() {
        "/usr/share/fonts/TTF/DejaVuSans.ttf"
    } else {
        FONT_PATH
    };
    let mut font_file = File::open(font_path).or_else(|_| File::open(FONT_PATH)).map_err(|_| format!("Не удалось найти шрифт."))?;
    let mut font_data = Vec::new();
    font_file.read_to_end(&mut font_data)?;
    let font = FontRef::try_from_slice(&font_data)?;

    let (conn, screen_num) = RustConnection::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let width = screen.width_in_pixels;
    let screen_height = screen.height_in_pixels;

    let atoms = Atoms::new(&conn)?;
    let win_id = conn.generate_id()?;
    let gc_id = conn.generate_id()?;
    let pixmap_id = conn.generate_id()?;

    let y_pos = (screen_height - PANEL_HEIGHT) as i16;

    // !!! ИЗМЕНЕНИЕ 1: Добавляем EventMask::LEAVE_WINDOW !!!
    let win_values = CreateWindowAux::new()
        .background_pixel(BG_COLOR)
        .event_mask(EventMask::EXPOSURE | EventMask::PROPERTY_CHANGE | EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION | EventMask::LEAVE_WINDOW);

    conn.create_window(
        screen.root_depth, win_id, root,
        0, y_pos, width, PANEL_HEIGHT, 0,
        WindowClass::INPUT_OUTPUT, screen.root_visual, &win_values,
    )?;

    // ... (ОСТАЛЬНАЯ НАСТРОЙКА ОКНА БЕЗ ИЗМЕНЕНИЙ) ...
    conn.create_pixmap(screen.root_depth, pixmap_id, win_id, width, PANEL_HEIGHT)?;
    conn.change_property32(PropMode::REPLACE, win_id, atoms._net_wm_window_type, atoms.atom, &[atoms._net_wm_window_type_dock])?;
    conn.change_property32(PropMode::REPLACE, win_id, atoms._net_wm_desktop, atoms.cardinal, &[0xFFFFFFFF])?;

    let mut hints_data = vec![0u32; 18];
    hints_data[0] = 12;
    conn.change_property32(PropMode::REPLACE, win_id, atoms.wm_normal_hints, AtomEnum::WM_SIZE_HINTS, &hints_data)?;

    let struts_partial = [0, 0, 0, PANEL_HEIGHT as u32, 0, 0, 0, 0, 0, 0, 0, width as u32];
    conn.change_property32(PropMode::REPLACE, win_id, atoms._net_wm_strut_partial, atoms.cardinal, &struts_partial)?;

    let gc_values = CreateGCAux::new().foreground(TEXT_COLOR).background(BG_COLOR);
    conn.create_gc(gc_id, win_id, &gc_values)?;

    conn.set_selection_owner(win_id, atoms.net_system_tray_s0, CURRENT_TIME)?;
    let tray_msg = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32, sequence: 0, window: root,
        type_: atoms.manager,
        data: [CURRENT_TIME, atoms.net_system_tray_s0, win_id, 0, 0].into(),
    };
    conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, tray_msg)?;

    conn.map_window(win_id)?;
    conn.configure_window(win_id, &ConfigureWindowAux::new().x(0).y(y_pos as i32))?;

    let root_values = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    conn.change_window_attributes(root, &root_values)?;

    let mut app = AppState {
        conn, atoms, screen_num, win_id, pixmap_id, gc_id, width,
        tray_icons: Vec::new(),
        click_regions: Vec::new(),
        font,
        mouse_x: -1,
        window_cache: HashMap::new(),
        hovered_window: None,
        render_buffer: Vec::with_capacity(2048),
        raw_windows_buf: Vec::with_capacity(64), // Инициализация буфера
    };

    redraw(&mut app)?;

    let mut last_time_str = Local::now().format("%H:%M").to_string();
    let fd = app.conn.stream().as_raw_fd();

    loop {
        app.conn.flush()?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let nanos = now.subsec_nanos();
        let millis_until_next_sec = (1000 - (nanos / 1_000_000)) as i32;
        let timeout = millis_until_next_sec + 10;

        unsafe {
            let mut poll_fd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            libc::poll(&mut poll_fd, 1, timeout);
        }

        let mut should_redraw = false;

        while let Some(event) = app.conn.poll_for_event()? {
            match event {
                Event::Expose(e) if e.window == win_id && e.count == 0 => { should_redraw = true; },
                Event::PropertyNotify(e) => {
                    if e.atom == app.atoms._net_client_list || e.atom == app.atoms._net_active_window {
                        should_redraw = true;
                    }
                    else if e.atom == app.atoms._net_wm_name || e.atom == AtomEnum::WM_NAME.into() || e.atom == app.atoms._net_wm_icon {
                        app.window_cache.remove(&e.window);
                        should_redraw = true;
                    }
                },
                Event::ButtonPress(e) => {
                    handle_click(&app, e.event_x, e.detail)?;
                    should_redraw = true;
                },
                Event::MotionNotify(e) => {
                    if e.event_x != app.mouse_x {
                        app.mouse_x = e.event_x;
                        let new_hovered = get_hovered_window(&app, e.event_x);
                        // Логика здесь уже хорошая: перерисовка ТОЛЬКО если сменилось окно под мышкой
                        if new_hovered != app.hovered_window {
                            app.hovered_window = new_hovered;
                            should_redraw = true;
                        }
                    }
                },
                // !!! ИЗМЕНЕНИЕ 2: Обработка ухода мыши с окна !!!
                Event::LeaveNotify(e) => {
                    // Проверяем detail != NotifyInferior, чтобы не сбрасывать hover,
                    // если мышь перешла на дочернее окно (например, иконку в трее, если она внутри панели)
                    if e.event == win_id && e.detail != NotifyDetail::INFERIOR {
                        if app.hovered_window.is_some() {
                            app.hovered_window = None;
                            app.mouse_x = -1; // Сброс позиции X
                            should_redraw = true;
                        }
                    }
                },
                Event::ClientMessage(e) if e.type_ == app.atoms._net_system_tray_opcode => {
                    let data = e.data.as_data32();
                    if data[1] == 0 { handle_docking(&mut app, data[2])?; should_redraw = true; }
                }
                Event::DestroyNotify(e) => {
                    if let Some(pos) = app.tray_icons.iter().position(|&w| w == e.window) {
                        app.tray_icons.remove(pos);
                        should_redraw = true;
                    }
                    if app.window_cache.contains_key(&e.window) {
                        app.window_cache.remove(&e.window);
                        should_redraw = true;
                    }
                }
                _ => {}
            }
        }

        let current_time_str = Local::now().format("%H:%M").to_string();
        if current_time_str != last_time_str {
            last_time_str = current_time_str;
            should_redraw = true;
        }

        if should_redraw {
            redraw(&mut app)?;
        }
    }
}

// ... (get_hovered_window и fetch_window_data БЕЗ ИЗМЕНЕНИЙ) ...
fn get_hovered_window(app: &AppState, mouse_x: i16) -> Option<Window> {
    for (start, end, win) in &app.click_regions {
        if mouse_x >= *start && mouse_x <= *end {
            return Some(*win);
        }
    }
    None
}

fn fetch_window_data(conn: &RustConnection, atoms: &Atoms, win: Window) -> CachedWindowData {
    let utf_cookie = conn.get_property(false, win, atoms._net_wm_name, atoms.utf8_string, 0, 1024).ok();
    let str_cookie = conn.get_property(false, win, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024).ok();

    let mut title = String::new();
    if let Some(cookie) = utf_cookie {
        if let Ok(r) = cookie.reply() {
            if r.value_len > 0 { title = String::from_utf8_lossy(&r.value).to_string(); }
        }
    }
    if title.is_empty() {
        if let Some(cookie) = str_cookie {
            if let Ok(r) = cookie.reply() { title = String::from_utf8_lossy(&r.value).to_string(); }
        }
    }

    let sanitized_title: String = title.chars()
        .filter(|c| !c.is_control() && match *c {
            '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{feff}' => false,
            _ => true
        })
        .collect();

    let mut icon_buffer = None;
    let mut icon_w = 0;
    let mut icon_h = 0;

    if let Ok(cookie) = conn.get_property(false, win, atoms._net_wm_icon, atoms.cardinal, 0, 100000) {
        if let Ok(reply) = cookie.reply() {
            if reply.value_len > 0 {
                if let Some(data_iter) = reply.value32() {
                    let data: Vec<u32> = data_iter.collect();
                    let mut best_start = 0;
                    let mut best_w = 0;
                    let mut max_w = 0;
                    let mut max_start = 0;
                    let mut idx = 0;
                    while idx + 2 < data.len() {
                        let w = data[idx] as usize;
                        let h = data[idx+1] as usize;
                        let size = w * h;
                        if idx + 2 + size > data.len() { break; }
                        if w > max_w { max_w = w; max_start = idx + 2; }
                        if w >= ICON_SIZE as usize {
                            if best_w == 0 || w < best_w { best_w = w; best_start = idx + 2; }
                        }
                        idx += 2 + size;
                    }
                    if best_w == 0 && max_w > 0 { best_w = max_w; best_start = max_start; }

                    if best_w > 0 {
                        let src_w = best_w;
                        let src_h = data[best_start - 1] as usize;
                        let pixels = &data[best_start..best_start + (src_w * src_h)];
                        let mut img_buf: RgbaImage = ImageBuffer::new(src_w as u32, src_h as u32);
                        for (i, &px) in pixels.iter().enumerate() {
                            let x_px = (i % src_w) as u32;
                            let y_px = (i / src_w) as u32;
                            img_buf.put_pixel(x_px, y_px, image::Rgba([
                                ((px >> 16) & 0xFF) as u8,
                                ((px >> 8) & 0xFF) as u8,
                                (px & 0xFF) as u8,
                                ((px >> 24) & 0xFF) as u8,
                            ]));
                        }
                        let resized = image::imageops::resize(&img_buf, ICON_SIZE as u32, ICON_SIZE as u32, FilterType::Lanczos3);
                        icon_buffer = Some(resized.into_raw());
                        icon_w = ICON_SIZE;
                        icon_h = ICON_SIZE;
                    }
                }
            }
        }
    }

    CachedWindowData {
        title: sanitized_title,
        icon_buffer,
        icon_width: icon_w,
        icon_height: icon_h,
    }
}

// !!! ИЗМЕНЕНИЕ 3: Оптимизация redraw (использование общего буфера векторов) !!!
fn redraw(app: &mut AppState) -> Result<(), Box<dyn std::error::Error>> {
    let draw_target = app.pixmap_id;

    let rect = Rectangle { x: 0, y: 0, width: app.width, height: PANEL_HEIGHT };
    app.conn.change_gc(app.gc_id, &ChangeGCAux::new().foreground(BG_COLOR))?;
    app.conn.poly_fill_rectangle(draw_target, app.gc_id, &[rect])?;
    app.conn.change_gc(app.gc_id, &ChangeGCAux::new().foreground(TEXT_COLOR))?;

    app.click_regions.clear();

    let time_str = Local::now().format("%H:%M").to_string();
    let date_str = Local::now().format("%Y-%m-%d").to_string();
    let tray_w = (app.tray_icons.len() as u16) * TRAY_ICON_WIDTH;

    let time_width = calculate_text_width(&app.font, FONT_SIZE_MAIN, &time_str);
    let date_width = calculate_text_width(&app.font, FONT_SIZE_DATE, &date_str);
    let max_text_width = if time_width > date_width { time_width } else { date_width };

    let clock_x_start = app.width as i16 - (max_text_width as i16 + 8);
    let tray_start_x = clock_x_start - (tray_w as i16 + 15);

    let time_x_offset = if time_width < max_text_width { (max_text_width - time_width) / 2.0 } else { 0.0 };
    draw_text_render(&app.conn, draw_target, app.gc_id, &app.font, &mut app.render_buffer, &time_str, FONT_SIZE_MAIN, clock_x_start + time_x_offset as i16, 2, BG_COLOR, TEXT_COLOR)?;

    let date_x_offset = if date_width < max_text_width { (max_text_width - date_width) / 2.0 } else { 0.0 };
    draw_text_render(&app.conn, draw_target, app.gc_id, &app.font, &mut app.render_buffer, &date_str, FONT_SIZE_DATE, clock_x_start + date_x_offset as i16, 20, BG_COLOR, DATE_COLOR)?;

    for (i, &win) in app.tray_icons.iter().enumerate() {
        let x = tray_start_x + (i as i16 * TRAY_ICON_WIDTH as i16);
        let y_tray = (PANEL_HEIGHT - 24) / 2;
        app.conn.configure_window(win, &ConfigureWindowAux::new().x(x as i32).y(y_tray as i32).width(24).height(24))?;
        app.conn.map_window(win)?;
    }

    let window_area_limit = tray_start_x - 10;
    let available_width_for_windows = window_area_limit as f32;

    let root = app.conn.setup().roots[app.screen_num].root;
    let client_cookie = app.conn.get_property(false, root, app.atoms._net_client_list, AtomEnum::ANY, 0, 1024)?;
    let active_cookie = app.conn.get_property(false, root, app.atoms._net_active_window, AtomEnum::ANY, 0, 1)?;

    let active_win = active_cookie.reply().ok()
        .and_then(|r| r.value32().and_then(|mut i| i.next()))
        .unwrap_or(0);

    // ОПТИМИЗАЦИЯ: Используем буфер из AppState вместо создания нового вектора
    app.raw_windows_buf.clear();
    if let Ok(reply) = client_cookie.reply() {
        if let Some(list) = reply.value32() {
            // extend вместо создания нового Vec
            app.raw_windows_buf.extend(list);
        }
    }
    // Фолбек для окон без _NET_CLIENT_LIST
    if app.raw_windows_buf.is_empty() {
        if let Ok(tree) = app.conn.query_tree(root)?.reply() {
            for w in tree.children {
                 let state = app.conn.get_property(false, w, app.atoms.wm_state, AtomEnum::ANY, 0, 1)?.reply();
                 if let Ok(r) = state { if r.value_len > 0 { app.raw_windows_buf.push(w); } }
            }
        }
    }

    struct WindowDrawData<'a> {
        win: Window,
        data: &'a CachedWindowData,
        ideal_width: f32,
        is_active: bool,
    }

    let mut visible_windows: Vec<WindowDrawData> = Vec::new();
    let mut total_ideal_width: f32 = 0.0;

    // Итерация по кешированному вектору
    for &w in &app.raw_windows_buf {
        if !app.window_cache.contains_key(&w) {
            let type_cookie = app.conn.get_property(false, w, app.atoms._net_wm_window_type, AtomEnum::ATOM, 0, 1024).ok();
            let mut is_dock = false;
            if let Some(cookie) = type_cookie {
                if let Ok(reply) = cookie.reply() {
                    if let Some(mut atoms_iter) = reply.value32() {
                        if atoms_iter.any(|a|
                            a == app.atoms._net_wm_window_type_dock ||
                            a == app.atoms._net_wm_window_type_desktop ||
                            a == app.atoms._net_wm_window_type_splash
                        ) { is_dock = true; }
                    }
                }
            }
            if !is_dock {
                app.conn.change_window_attributes(w, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE)).ok();
                let data = fetch_window_data(&app.conn, &app.atoms, w);
                app.window_cache.insert(w, data);
            }
        }
    }

    for &w in &app.raw_windows_buf {
        if w == app.win_id || app.tray_icons.contains(&w) { continue; }

        if let Some(data) = app.window_cache.get(&w) {
            let class_str = data.title.to_lowercase();
            if class_str.contains("conky") || class_str.contains("tint2") || class_str.contains("plank") { continue; }

            let text_width = calculate_text_width(&app.font, FONT_SIZE_MAIN, &data.title);
            let max_window_width = 250.0;
            let calc_width = (ICON_SIZE as f32 + text_width + 24.0).min(max_window_width);

            total_ideal_width += calc_width;

            visible_windows.push(WindowDrawData {
                win: w,
                data,
                ideal_width: calc_width,
                is_active: w == active_win,
            });
        }
    }

    let window_count = visible_windows.len();
    if window_count == 0 {
        app.conn.copy_area(app.pixmap_id, app.win_id, app.gc_id, 0, 0, 0, 0, app.width, PANEL_HEIGHT)?;
        app.conn.flush()?;
        return Ok(());
    }

    let use_compression = total_ideal_width > available_width_for_windows;
    let fixed_width_per_window = if use_compression {
        (available_width_for_windows / window_count as f32).floor()
    } else { 0.0 };

    let mut current_x: i16 = 0;

    for win_data in visible_windows {
        let actual_width = if use_compression { fixed_width_per_window as i16 } else { win_data.ideal_width as i16 };

        let mut bg = BG_COLOR;
        if win_data.is_active {
            bg = ACTIVE_BG_COLOR;
            app.conn.change_gc(app.gc_id, &ChangeGCAux::new().foreground(ACTIVE_BG_COLOR))?;
            app.conn.poly_fill_rectangle(draw_target, app.gc_id, &[Rectangle{x: current_x, y: 2, width: actual_width as u16, height: PANEL_HEIGHT-4}])?;

            app.conn.change_gc(app.gc_id, &ChangeGCAux::new().foreground(UNDERLINE_COLOR))?;
            app.conn.poly_fill_rectangle(draw_target, app.gc_id, &[Rectangle{
                x: current_x,
                y: (PANEL_HEIGHT - UNDERLINE_HEIGHT - 2) as i16,
                width: actual_width as u16,
                height: UNDERLINE_HEIGHT
            }])?;
        } else {
            if Some(win_data.win) == app.hovered_window {
                bg = HOVER_BG_COLOR;
                app.conn.change_gc(app.gc_id, &ChangeGCAux::new().foreground(HOVER_BG_COLOR))?;
                app.conn.poly_fill_rectangle(draw_target, app.gc_id, &[Rectangle{x: current_x, y: 2, width: actual_width as u16, height: PANEL_HEIGHT-4}])?;
            }
        }

        if actual_width >= (ICON_SIZE as i16 + 6) {
             if let Some(ref pixels) = win_data.data.icon_buffer {
                 draw_icon_fast(&app.conn, draw_target, app.gc_id, pixels, win_data.data.icon_width, win_data.data.icon_height, current_x + 6, ICON_Y_OFFSET, bg, &mut app.render_buffer)?;
             }
        }

        let text_area_w = actual_width - (ICON_SIZE as i16 + 18);
        if text_area_w > 10 {
            let display_text = if use_compression || calculate_text_width(&app.font, FONT_SIZE_MAIN, &win_data.data.title) > text_area_w as f32 {
                shorten_text_to_fit(&app.font, FONT_SIZE_MAIN, &win_data.data.title, text_area_w as f32)
            } else {
                win_data.data.title.clone()
            };

            draw_text_render(&app.conn, draw_target, app.gc_id, &app.font, &mut app.render_buffer, &display_text, FONT_SIZE_MAIN, current_x + ICON_SIZE as i16 + 14, TEXT_Y_OFFSET, bg, TEXT_COLOR)?;
        }

        app.click_regions.push((current_x, current_x + actual_width, win_data.win));
        current_x += actual_width;
    }

    app.conn.copy_area(app.pixmap_id, app.win_id, app.gc_id, 0, 0, 0, 0, app.width, PANEL_HEIGHT)?;
    app.conn.flush()?;
    Ok(())
}

// ... (ВСЕ ОСТАЛЬНЫЕ ФУНКЦИИ БЕЗ ИЗМЕНЕНИЙ) ...
// draw_icon_fast, draw_text_render, calculate_text_width, shorten_text_to_fit, layout_paragraph, handle_click, handle_docking, Atoms
// ... Вставьте их сюда ...
fn draw_icon_fast(
    conn: &RustConnection, target: Drawable, gc: Gcontext,
    pixels: &[u8], width: u16, height: u16,
    x: i16, y: i16, bg_color: u32,
    render_buf: &mut Vec<u8>
) -> Result<(), Box<dyn std::error::Error>> {

    render_buf.clear();
    let bg_r = ((bg_color >> 16) & 0xFF) as u16;
    let bg_g = ((bg_color >> 8) & 0xFF) as u16;
    let bg_b = (bg_color & 0xFF) as u16;

    for chunk in pixels.chunks(4) {
        let r = chunk[0] as u16;
        let g = chunk[1] as u16;
        let b = chunk[2] as u16;
        let a = chunk[3] as u16;

        let inv_alpha = 255 - a;

        let out_r = ((r * a + bg_r * inv_alpha) >> 8) as u8;
        let out_g = ((g * a + bg_g * inv_alpha) >> 8) as u8;
        let out_b = ((b * a + bg_b * inv_alpha) >> 8) as u8;

        render_buf.push(out_b);
        render_buf.push(out_g);
        render_buf.push(out_r);
        render_buf.push(0xFF);
    }

    conn.put_image(ImageFormat::Z_PIXMAP, target, gc, width, height, x, y, 0, 24, &render_buf)?;
    Ok(())
}

fn draw_text_render(
    conn: &RustConnection,
    target: Drawable,
    gc: Gcontext,
    font: &FontRef,
    render_buf: &mut Vec<u8>,
    text: &str,
    font_size: f32, x: i16, y: i16, bg_color: u32, fg_color: u32
) -> Result<(), Box<dyn std::error::Error>> {
    if text.is_empty() { return Ok(()); }

    let scale = PxScale::from(font_size);
    let scaled_font = font.as_scaled(scale);

    let mut glyphs = Vec::new();
    layout_paragraph(scaled_font, point(0.0, 0.0), 9999.0, text, &mut glyphs);
    if glyphs.is_empty() { return Ok(()); }

    let height = (font_size.ceil() as usize) + 8;
    let width = glyphs.last().map(|g| g.position.x + scaled_font.h_advance(g.id)).unwrap_or(0.0).ceil() as usize + 4;
    if width == 0 { return Ok(()); }

    let buf_size = width * height * 4;
    if render_buf.capacity() < buf_size {
        render_buf.reserve(buf_size - render_buf.capacity());
    }
    render_buf.clear();
    render_buf.resize(buf_size, 0);

    let bg_r = ((bg_color >> 16) & 0xFF) as u8;
    let bg_g = ((bg_color >> 8) & 0xFF) as u8;
    let bg_b = (bg_color & 0xFF) as u8;

    let fg_r = ((fg_color >> 16) & 0xFF) as u16;
    let fg_g = ((fg_color >> 8) & 0xFF) as u16;
    let fg_b = (fg_color & 0xFF) as u16;

    for i in 0..(width * height) {
        render_buf[i * 4 + 0] = bg_b;
        render_buf[i * 4 + 1] = bg_g;
        render_buf[i * 4 + 2] = bg_r;
        render_buf[i * 4 + 3] = 0xFF;
    }

    for glyph in glyphs {
        if let Some(outlined) = scaled_font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, c| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px >= 0 && px < width as i32 && py >= 0 && py < height as i32 {
                    let idx = ((py as usize) * width + (px as usize)) * 4;

                    let alpha = (c * 256.0) as u16;
                    let inv_alpha = 256 - alpha;

                    let cur_b = render_buf[idx+0] as u16;
                    let cur_g = render_buf[idx+1] as u16;
                    let cur_r = render_buf[idx+2] as u16;

                    render_buf[idx+0] = ((fg_b * alpha + cur_b * inv_alpha) >> 8) as u8;
                    render_buf[idx+1] = ((fg_g * alpha + cur_g * inv_alpha) >> 8) as u8;
                    render_buf[idx+2] = ((fg_r * alpha + cur_r * inv_alpha) >> 8) as u8;
                }
            });
        }
    }

    conn.put_image(ImageFormat::Z_PIXMAP, target, gc, width as u16, height as u16, x, y, 0, 24, &render_buf)?;
    Ok(())
}

fn calculate_text_width(font: &FontRef, size: f32, text: &str) -> f32 {
    let scale = PxScale::from(size);
    let scaled_font = font.as_scaled(scale);
    let mut width = 0.0;
    let mut last_glyph_id = None;
    for c in text.chars() {
        let glyph_id = scaled_font.glyph_id(c);
        if let Some(last) = last_glyph_id {
            width += scaled_font.kern(last, glyph_id);
        }
        width += scaled_font.h_advance(glyph_id);
        last_glyph_id = Some(glyph_id);
    }
    width.ceil()
}

fn shorten_text_to_fit(font: &FontRef, size: f32, text: &str, max_width: f32) -> String {
    let ellipsis = "...";
    let ellipsis_width = calculate_text_width(font, size, ellipsis);

    if max_width < ellipsis_width {
        return String::new();
    }

    let target_width = max_width - ellipsis_width;
    let mut current_text = text.to_string();

    while !current_text.is_empty() {
        let w = calculate_text_width(font, size, &current_text);
        if w <= target_width {
            current_text.push_str(ellipsis);
            return current_text;
        }
        current_text.pop();
    }

    if ellipsis_width <= max_width { return ellipsis.to_string(); }
    String::new()
}

pub fn layout_paragraph<F, SF>(
    font: SF, position: ab_glyph::Point, _max_width: f32, text: &str, target: &mut Vec<ab_glyph::Glyph>,
) where F: Font, SF: ScaleFont<F>, {
    let v_advance = font.height() + font.line_gap();
    let mut caret = position + point(0.0, font.ascent());
    let mut last_glyph_id = None;
    for c in text.chars() {
        if c.is_control() {
            if c == '\n' { caret = point(position.x, caret.y + v_advance); last_glyph_id = None; }
            continue;
        }
        let mut glyph = font.scaled_glyph(c);
        if let Some(previous) = last_glyph_id { caret.x += font.kern(previous, glyph.id); }
        glyph.position = point(caret.x.round(), caret.y);
        last_glyph_id = Some(glyph.id);
        caret.x += font.h_advance(glyph.id);
        target.push(glyph);
    }
}

fn handle_click(app: &AppState, x: i16, button: u8) -> Result<(), Box<dyn std::error::Error>> {
    for (start, end, win) in &app.click_regions {
        if x >= *start && x <= *end {
            if button == 1 {
                let event = ClientMessageEvent {
                    response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 0, window: *win,
                    type_: app.atoms._net_active_window, data: [2, CURRENT_TIME, 0, 0, 0].into(),
                };
                app.conn.send_event(false, app.conn.setup().roots[app.screen_num].root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)?;
                app.conn.set_input_focus(InputFocus::POINTER_ROOT, *win, CURRENT_TIME)?;
            } else if button == 3 {
                let event = ClientMessageEvent {
                    response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 0, window: *win,
                    type_: app.atoms._net_close_window, data: [CURRENT_TIME, 2, 0, 0, 0].into(),
                };
                app.conn.send_event(false, app.conn.setup().roots[app.screen_num].root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)?;
            }
            app.conn.flush()?;
            return Ok(());
        }
    }
    Ok(())
}

fn handle_docking(app: &mut AppState, win: Window) -> Result<(), Box<dyn std::error::Error>> {
    if !app.tray_icons.contains(&win) {
        app.tray_icons.push(win);
        app.conn.reparent_window(win, app.win_id, 0, 0)?;
        app.conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY))?;
        app.conn.map_window(win)?;
    }
    Ok(())
}

struct Atoms {
    atom: Atom, cardinal: Atom, utf8_string: Atom, manager: Atom, wm_state: Atom,
    _net_wm_window_type: Atom, _net_wm_window_type_dock: Atom, _net_wm_strut_partial: Atom,
    _net_wm_strut: Atom, _net_wm_desktop: Atom, wm_normal_hints: Atom,
    _net_wm_window_type_desktop: Atom, _net_wm_window_type_splash: Atom,
    _net_client_list: Atom, _net_wm_name: Atom, _net_active_window: Atom, _net_wm_icon: Atom,
    _net_system_tray_opcode: Atom, net_system_tray_s0: Atom, _net_close_window: Atom,
}

impl Atoms {
    fn new(c: &RustConnection) -> Result<Self, Box<dyn std::error::Error>> {
        let i = |n| c.intern_atom(false, n).unwrap().reply().unwrap().atom;
        Ok(Self {
            atom: i(b"ATOM"), cardinal: i(b"CARDINAL"), utf8_string: i(b"UTF8_STRING"), manager: i(b"MANAGER"),
            wm_state: i(b"WM_STATE"),
            _net_wm_window_type: i(b"_NET_WM_WINDOW_TYPE"),
            _net_wm_window_type_dock: i(b"_NET_WM_WINDOW_TYPE_DOCK"),
            _net_wm_window_type_desktop: i(b"_NET_WM_WINDOW_TYPE_DESKTOP"),
            _net_wm_window_type_splash: i(b"_NET_WM_WINDOW_TYPE_SPLASH"),
            _net_wm_strut_partial: i(b"_NET_WM_STRUT_PARTIAL"),
            _net_wm_strut: i(b"_NET_WM_STRUT"),
            _net_wm_desktop: i(b"_NET_WM_DESKTOP"),
            wm_normal_hints: i(b"WM_NORMAL_HINTS"),
            _net_client_list: i(b"_NET_CLIENT_LIST"),
            _net_wm_name: i(b"_NET_WM_NAME"), _net_active_window: i(b"_NET_ACTIVE_WINDOW"), _net_wm_icon: i(b"_NET_WM_ICON"),
            _net_system_tray_opcode: i(b"_NET_SYSTEM_TRAY_OPCODE"), net_system_tray_s0: i(b"_NET_SYSTEM_TRAY_S0"),
            _net_close_window: i(b"_NET_CLOSE_WINDOW"),
        })
    }
}