use std::{fs::File, mem::size_of, path::Path};
use anyhow::{Context, Result};
use geo::Contains;
use memmap::Mmap;
use persy::{Config, Persy, PersyId, ValueIter};
use crate::AppConf;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionJson {
    pub code: u32,
    pub parent_code: u32,
    pub level: u32,
    pub center: String,
    pub name: String,
    pub polyline: String,
}

type Coord = geo::Coord<f32>;
type LineString = geo::LineString<f32>;
type Polygon = geo::Polygon<f32>;

#[derive(Debug, PartialEq)]
pub struct Region {
    pub code: u32,
    pub parent_code: u32,
    pub level: u32,
    pub center: Coord,
    pub name: String,
    pub polygons: Vec<Polygon>,
}
#[derive(Debug, serde::Serialize)]
pub struct Point {
    pub lng: f32,
    pub lat: f32,
}

#[derive(Debug, serde::Serialize)]
pub struct RegionInfo {
    pub code: u32,
    pub name: String,
    pub names: Vec<String>,
    pub center: Point,
}

const SEG_NAME: &str = "data";
const INDEX_NAME: &str = "code";
const TOP_INDEX_NAME: &str = "tcode";
const PARENT_INDEX_NAME: &str = "pcode";

static mut DB_INST: Option<Persy> = None;

pub fn init_db() -> Result<()> {
    let path = Path::new(&AppConf::get().database);
    let db = Persy::open(path, Config::new())
        .with_context(|| format!("无法打开数据库{}", AppConf::get().database))?;
    unsafe {
        debug_assert!(DB_INST.is_none());
        DB_INST = Some(db);
     }
    Ok(())
}

pub fn resolve_region(lng: f32, lat: f32) -> Result<Option<RegionInfo>> {
    let regions = find_region(lng, lat)?;
    let mut code = 0;
    let mut name = String::with_capacity(128);
    let mut names = Vec::with_capacity(4);
    let mut region = None;
    for item in &regions {
        code = item.code;
        name.push_str(&item.name);
        names.push(item.name.clone());
        region = Some(item);
    }
    if code == 0 {
        Ok(None)
    } else {
        let r = region.unwrap();
        let center = Point { lng: r.center.x, lat: r.center.y };
        Ok(Some(RegionInfo { code, name, names, center }))
    }
}

pub fn find_region(lng: f32, lat: f32) -> Result<Vec<Region>> {
    let db = get_db();
    let coord = Coord {x: lng, y: lat};
    let mut result = Vec::with_capacity(4);

    let iter = db.range::<u32, PersyId, _>(TOP_INDEX_NAME, 0..u32::MAX)
        .context("数据库范围检索失败")?;
    for (_, id_iter) in iter {
        let r1 = find_by_code(db, id_iter, &coord)?;
        if let Some(r) = r1 {
            let mut pcode = r.code;
            result.push(r);
            loop {
                let id_iter = db.get::<_, PersyId>(PARENT_INDEX_NAME, &pcode)?;
                match find_by_code(db, id_iter, &coord)? {
                    Some(r) => {
                        let scode = r.code;
                        result.push(r);
                        if pcode == scode {
                            break;
                        } else {
                           pcode = scode;
                        }
                    }
                    None => break,
                }
            }
            break;
        }
    }

    Ok(result)
}

pub fn convert(ac: &AppConf) {
    println!("正在读取json文件: {} ...", ac.convert);
    let region_json_vec: Vec<RegionJson> = {
        let path = Path::new(&ac.convert);
        let file = File::open(path).unwrap();
        let mmap = unsafe { Mmap::map(&file).unwrap() };
        serde_json::from_slice(&mmap).unwrap()
    };

    let g = Polygon::new(LineString::new(Vec::new()), Vec::new());
    geo::Contains::<Coord>::contains(&g, &Coord { x: 0.0, y: 0.0 });
    println!("完成读取，记录总数：{}", region_json_vec.len());

    let path = Path::new(&ac.database);
    Persy::create(path).unwrap();
    let db = Persy::open(path, Config::new()).unwrap();
    let mut tx = db.begin().unwrap();
    tx.create_segment(SEG_NAME).unwrap();
    tx.create_index::<u32, PersyId>(INDEX_NAME, persy::ValueMode::Cluster).unwrap();
    tx.create_index::<u32, PersyId>(PARENT_INDEX_NAME, persy::ValueMode::Cluster).unwrap();
    tx.create_index::<u32, PersyId>(TOP_INDEX_NAME, persy::ValueMode::Cluster).unwrap();
    tx.commit().unwrap();
    let mut tx = db.begin().unwrap();
    for rj in &region_json_vec {
        let r = Region {
            code: rj.code,
            parent_code: rj.parent_code,
            level: rj.level,
            center: str_to_coord(&rj.center),
            name: rj.name.clone(),
            polygons: str_to_polygons(&rj.polyline),
        };
        let bs = r.to_bytes();
        let id = tx.insert(SEG_NAME, &bs).unwrap();
        let bs2 = bs.clone();
        if r != Region::from_bytes(bs2) {
            panic!("错误，对象序列化不相等");
        }

        tx.put(INDEX_NAME, r.code, id).unwrap();
        tx.put(PARENT_INDEX_NAME, r.parent_code, id).unwrap();
        if r.code % 10000 == 0 {
            tx.put(TOP_INDEX_NAME, r.code, id).unwrap();
        }
    }
    tx.commit().unwrap();
    println!("生成{}数据库完成", ac.database);
}

fn str_to_coord(s: &str) -> Coord {
    let mut sp = s.split(',');
    Coord {
        x: sp.next().unwrap().parse().unwrap(),
        y: sp.next().unwrap().parse().unwrap(),
    }
}

fn str_to_linestring(s: &str) -> LineString {
    let mut coords = Vec::new();
    for s in s.split(';') {
        coords.push(str_to_coord(s));
    }
    LineString::new(coords)
}

fn str_to_polygons(s: &str) -> Vec<Polygon> {
    let mut result = Vec::new();
    for item in s.split('|') {
        result.push(Polygon::new(str_to_linestring(item), Vec::with_capacity(0)));
    }
    result
}

fn find_by_code(db: &Persy, iter: ValueIter<PersyId>, coord: &Coord)
    -> Result<Option<Region>>
{
    for rid in iter {
        let data = db.read(SEG_NAME, &rid).context("数据库读取记录内容失败")?;
        if let Some(data) = data {
            let region = Region::from_bytes(data);
            for item in &region.polygons {
                if Contains::<Coord>::contains(item, coord) {
                    return Ok(Some(region));
                }
            }

        }
    }
    Ok(None)
}

fn get_db() -> &'static Persy {
    unsafe {
        debug_assert!(DB_INST.is_some());
        match &DB_INST {
            Some(v) => v,
            None => std::hint::unreachable_unchecked()
        }
    }
}

impl Region {
    pub fn to_bytes(&self) -> Vec<u8> {
        let total = self.total_bytes();
        debug_assert!(total % 4 == 0);

        let mut out = Vec::<u8>::with_capacity(total);
        let out_ptr = out.as_mut_ptr() as *mut u32;

        unsafe {
            *out_ptr = (total as u32).to_le();
            *out_ptr.add(1) = self.code.to_le();
            *out_ptr.add(2) = self.parent_code.to_le();
            *out_ptr.add(3) = self.level.to_le();
            *out_ptr.add(4) = self.center.x.to_bits().to_le();
            *out_ptr.add(5) = self.center.y.to_bits().to_le();

            *out_ptr.add(6) = (self.name.len() as u32).to_le();
            let name_ptr = self.name.as_ptr() as *const u32;
            let name_u32_len = (self.name.len() + 3) / 4;
            for i in 0..name_u32_len {
                *out_ptr.add(7 + i) = *name_ptr.add(i);
            }

            let mut out_ptr = out_ptr.add(7 + name_u32_len);
            *out_ptr = (self.polygons.len() as u32).to_le();
            out_ptr = out_ptr.add(1);
            for item in &self.polygons {
                out_ptr = Self::write_linestring(out_ptr, item.exterior());
            }

            out.set_len(total);
        }

        out
    }

    pub fn from_bytes(input: Vec<u8>) -> Self {
        let in_ptr = input.as_ptr() as *const u32;

        unsafe {
            let total = (*in_ptr).to_le();
            debug_assert!(total % 4 == 0);

            let code = (*in_ptr.add(1)).to_le();
            let parent_code = (*in_ptr.add(2)).to_le();
            let level = (*in_ptr.add(3)).to_le();
            let center = Coord {
                x: f32::from_bits((*in_ptr.add(4)).to_le()),
                y: f32::from_bits((*in_ptr.add(5)).to_le()),
            };

            let name_len = (*in_ptr.add(6)).to_le() as usize;
            let name_u32_len = (name_len + 3) / 4;
            let mut name_vec = Vec::<u8>::with_capacity((name_len + 3) / 4 * 4);
            let name_ptr = name_vec.as_mut_ptr() as *mut u32;
            for i in 0..name_u32_len {
                *name_ptr.add(i) = (*in_ptr.add(7 + i)).to_le();
            }
            name_vec.set_len(name_len);
            let name = String::from_utf8_unchecked(name_vec);

            let mut in_ptr = in_ptr.add(7 + name_u32_len);
            let polygons_len = (*in_ptr).to_le() as usize;
            in_ptr = in_ptr.add(1);
            let mut polygons = Vec::<Polygon>::with_capacity(polygons_len);
            for _ in 0..polygons_len {
                let (in_ptr2, exterior) = Self::read_linestring(in_ptr);
                polygons.push(Polygon::new(exterior, Vec::with_capacity(0)));
                in_ptr = in_ptr2;

            }

            Region {
                code,
                parent_code,
                level,
                center,
                name,
                polygons,
            }
        }
    }

    unsafe fn write_linestring(out_ptr: *mut u32, val: &LineString) -> *mut u32 {
        *out_ptr = (val.0.len() as u32).to_le();
        let mut out_ptr = out_ptr.add(1);
        for item in &val.0 {
            *out_ptr = item.x.to_bits().to_le();
            out_ptr = out_ptr.add(1);
            *out_ptr = item.y.to_bits().to_le();
            out_ptr = out_ptr.add(1);
        }
        out_ptr
    }

    unsafe fn read_linestring(mut in_ptr: *const u32) -> (*const u32, LineString) {
        let vec_len = (*in_ptr).to_le() as usize;
        in_ptr = in_ptr.add(1);
        let mut coords = Vec::<Coord>::with_capacity(vec_len);
        let coords_ptr = coords.as_mut_ptr();
        for i in 0..vec_len {
            *coords_ptr.add(i) = Coord {
                x: f32::from_bits((*in_ptr).to_le()),
                y: f32::from_bits((*in_ptr.add(1)).to_le()),
            };
            in_ptr = in_ptr.add(2);
        }
        coords.set_len(vec_len);

        (in_ptr, LineString::new(coords))
    }

    fn total_bytes(&self) -> usize {
        const U32_SIZE: usize = size_of::<u32>();
        const COORD_SIZE: usize = size_of::<f32>() * 2;

        let mut total = U32_SIZE + U32_SIZE * 3 + COORD_SIZE;
        total += U32_SIZE + (self.name.len() + 3) / 4 * 4;
        total += U32_SIZE; // polygons len
        for item in &self.polygons {
            total += U32_SIZE + item.exterior().0.len() * COORD_SIZE;
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_read_write() {
        let s = Region {
            code: 12345,
            parent_code: 54321,
            level: 789,
            center: Coord{x: 110.434_13, y: 19.972767},
            name: "海口市".to_string(),
            polygons: vec![Polygon::new(
                LineString::new(vec![
                    Coord{x: 110.434_13, y: 19.972767},
                    Coord{x: 110.434_13, y: 20.972767},
                    Coord{x: 110.444_13, y: 20.972767},
                    Coord{x: 110.434_13, y: 19.972767},
                ]),
                Vec::with_capacity(0),
            )],
        };

        let bs = s.to_bytes();
        let d = Region::from_bytes(bs);
        assert_eq!(s, d);
    }
}
