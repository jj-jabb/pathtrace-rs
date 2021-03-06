use camera::Camera;
use collision::{Ray, RayHit, Sphere};
use material::Material;
use rand::{weak_rng, Rng, SeedableRng, XorShiftRng};
use rayon::prelude::*;
use std::f32;
use std::sync::atomic::{AtomicUsize, Ordering};

use vmath::{normalize, vec3, Vec3};

#[derive(Copy, Clone)]
pub struct Params {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_depth: u32,
    pub random_seed: bool,
}

pub struct Scene {
    spheres: Vec<Sphere>,
    materials: Vec<Material>,
    ray_count: AtomicUsize,
}

impl Scene {
    pub fn new(sphere_materials: &[(Sphere, Material)]) -> Scene {
        let (spheres, materials) = sphere_materials.iter().cloned().unzip();
        Scene {
            spheres,
            materials,
            ray_count: AtomicUsize::new(0),
        }
    }

    fn ray_hit(&self, ray: &Ray, t_min: f32, t_max: f32) -> Option<(RayHit, &Material)> {
        let mut result = None;
        let mut closest_so_far = t_max;
        for (sphere, material) in self.spheres.iter().zip(self.materials.iter()) {
            if let Some(ray_hit) = sphere.hit(ray, t_min, closest_so_far) {
                closest_so_far = ray_hit.t;
                result = Some((ray_hit, material));
            }
        }
        result
    }

    fn ray_trace(&self, ray_in: &Ray, depth: u32, max_depth: u32, rng: &mut XorShiftRng, ray_count: &mut usize) -> Vec3 {
        const MAX_T: f32 = f32::MAX;
        const MIN_T: f32 = 0.001;
        *ray_count += 1;
        if let Some((ray_hit, material)) = self.ray_hit(ray_in, MIN_T, MAX_T) {
            if depth < max_depth {
                if let Some((attenuation, scattered)) = material.scatter(ray_in, &ray_hit, rng) {
                    return attenuation * self.ray_trace(&scattered, depth + 1, max_depth, rng, ray_count);
                }
            }
            return Vec3::zero();
        } else {
            let unit_direction = normalize(ray_in.direction);
            let t = 0.5 * (unit_direction.y + 1.0);
            (1.0 - t) * vec3(1.0, 1.0, 1.0) + t * vec3(0.5, 0.7, 1.0)
        }
    }

    pub fn update(
        &self,
        params: &Params,
        camera: &Camera,
        frame_num: u32,
        buffer: &mut [(f32, f32, f32)],
    ) -> usize {
        self.ray_count.store(0, Ordering::Relaxed);

        let inv_nx = 1.0 / params.width as f32;
        let inv_ny = 1.0 / params.height as f32;
        let inv_ns = 1.0 / params.samples as f32;

        let mix_prev = frame_num as f32 / (frame_num + 1) as f32;
        let mix_new = 1.0 - mix_prev;

        // parallel iterate each row of pixels
        buffer
            .par_chunks_mut(params.width as usize)
            .enumerate()
            .for_each(|(j, row)| {
                let mut ray_count = 0;
                let mut rng = if params.random_seed {
                    weak_rng()
                } else {
                    let state = (j as u32 * 9781 + frame_num * 6271) | 1;
                    XorShiftRng::from_seed([state, state, state, state])
                };
                row.iter_mut().enumerate().for_each(|(i, color_out)| {
                    let mut col = vec3(0.0, 0.0, 0.0);
                    for _ in 0..params.samples {
                        let u = (i as f32 + rng.next_f32()) * inv_nx;
                        let v = (j as f32 + rng.next_f32()) * inv_ny;
                        let ray = camera.get_ray(u, v, &mut rng);
                        col += self.ray_trace(&ray, 0, params.max_depth, &mut rng, &mut ray_count);
                    }
                    col *= inv_ns;
                    color_out.0 = color_out.0 * mix_prev + col.x * mix_new;
                    color_out.1 = color_out.1 * mix_prev + col.y * mix_new;
                    color_out.2 = color_out.2 * mix_prev + col.z * mix_new;
                });
                self.ray_count.fetch_add(ray_count, Ordering::Relaxed);
            });
        self.ray_count.load(Ordering::Relaxed)
    }
}
