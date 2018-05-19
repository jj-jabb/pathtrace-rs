use material::Material;
use math::align_to;
use simd::{F32XN_LANES, ArrayF32xN};
use vmath::{vec3, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

#[inline]
pub fn ray(origin: Vec3, direction: Vec3) -> Ray {
    Ray { origin, direction }
}

impl Ray {
    #[inline]
    #[allow(dead_code)]
    pub fn new(origin: Vec3, direction: Vec3) -> Ray {
        Ray { origin, direction }
    }
    #[inline]
    pub fn point_at_parameter(&self, t: f32) -> Vec3 {
        self.origin + (t * self.direction)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub point: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub struct Sphere {
    pub centre: Vec3,
    pub radius: f32,
}

#[inline]
pub fn sphere(centre: Vec3, radius: f32, material: Material) -> (Sphere, Material) {
    (Sphere { centre, radius }, material)
}

#[derive(Debug)]
pub struct SpheresSoA {
    len: usize,
    centre_x: Vec<ArrayF32xN>,
    centre_y: Vec<ArrayF32xN>,
    centre_z: Vec<ArrayF32xN>,
    radius_sq: Vec<ArrayF32xN>,
    radius_inv: Vec<ArrayF32xN>,
    material: Vec<[Material; F32XN_LANES]>,
}

impl SpheresSoA {
    pub fn new(sphere_materials: &[(Sphere, Material)]) -> SpheresSoA {
        let unaligned_len = sphere_materials.len();
        let len = align_to(unaligned_len, F32XN_LANES) / F32XN_LANES;
        let mut centre_x = Vec::with_capacity(len);
        let mut centre_y = Vec::with_capacity(len);
        let mut centre_z = Vec::with_capacity(len);
        let mut radius_inv = Vec::with_capacity(len);
        let mut radius_sq = Vec::with_capacity(len);
        let mut material = Vec::with_capacity(len);
        for chunk in sphere_materials.chunks(F32XN_LANES) {
            let mut chunk_x = ArrayF32xN::zero();
            let mut chunk_y = ArrayF32xN::zero();
            let mut chunk_z = ArrayF32xN::zero();
            let mut chunk_rsq = ArrayF32xN::zero();
            let mut chunk_rinv = ArrayF32xN::zero();
            let mut chunk_mat = [Material::Invalid; F32XN_LANES];
            for (index, (sphere, mat)) in chunk.iter().enumerate() {
                chunk_x.0[index] = sphere.centre.x;
                chunk_y.0[index] = sphere.centre.y;
                chunk_z.0[index] = sphere.centre.z;
                chunk_rsq.0[index] = sphere.radius * sphere.radius;
                chunk_rinv.0[index] = 1.0 / sphere.radius;
                chunk_mat[index] = *mat;
            }
            centre_x.push(chunk_x);
            centre_y.push(chunk_y);
            centre_z.push(chunk_z);
            radius_sq.push(chunk_rsq);
            radius_inv.push(chunk_rinv);
            material.push(chunk_mat);
        }
        SpheresSoA {
            len,
            centre_x,
            centre_y,
            centre_z,
            radius_sq,
            radius_inv,
            material,
        }
    }

    pub fn hit(&self, ray: &Ray, t_min: f32, t_max: f32) -> Option<(RayHit, &Material)> {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            if is_x86_feature_detected!("sse4.1") {
                return unsafe { self.hit_sse4_1(ray, t_min, t_max) };
            }
            panic!("No implementation");
        }

        // self.hit_scalar(ray, t_min, t_max)
    }

    /*
    fn hit_scalar(&self, ray: &Ray, t_min: f32, t_max: f32) -> Option<(RayHit, &Material)> {
        let mut hit_t = t_max;
        let mut hit_index = self.len;
        for ((((index, centre_x), centre_y), centre_z), radius_sq) in self
            .centre_x
            .iter()
            .enumerate()
            .zip(self.centre_y.iter())
            .zip(self.centre_z.iter())
            .zip(self.radius_sq.iter())
        {
            let co = vec3(
                centre_x - ray.origin.x,
                centre_y - ray.origin.y,
                centre_z - ray.origin.z,
            );
            let nb = dot(co, ray.direction);
            let c = dot(co, co) - radius_sq;
            let discriminant = nb * nb - c;
            if discriminant > 0.0 {
                let discriminant_sqrt = discriminant.sqrt();
                let mut t = nb - discriminant_sqrt;
                if t < t_min {
                    t = nb + discriminant_sqrt;
                }
                if t > t_min && t < hit_t {
                    hit_t = t;
                    hit_index = index;
                }
            }
        }
        if hit_index < self.len {
            let point = ray.point_at_parameter(hit_t);
            let normal = vec3(
                point.x - self.centre_x[hit_index],
                point.y - self.centre_y[hit_index],
                point.z - self.centre_z[hit_index],
            ) * self.radius_inv[hit_index];
            let material = &self.material[hit_index];
            Some((RayHit { point, normal }, material))
        } else {
            None
        }
    }
    */

    #[cfg_attr(any(target_arch = "x86", target_arch = "x86_64"), target_feature(enable = "sse4.1"))]
    unsafe fn hit_sse4_1(&self, ray: &Ray, t_min: f32, t_max: f32) -> Option<(RayHit, &Material)> {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::*;
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::*;
        let t_min = _mm_set_ps1(t_min);
        let mut hit_t = _mm_set_ps1(t_max);
        let mut hit_index = _mm_set_epi32(-1, -1, -1, -1);
        // load ray origin
        let ro_x = _mm_set_ps1(ray.origin.x);
        let ro_y = _mm_set_ps1(ray.origin.y);
        let ro_z = _mm_set_ps1(ray.origin.z);
        // load ray direction
        let rd_x = _mm_set_ps1(ray.direction.x);
        let rd_y = _mm_set_ps1(ray.direction.y);
        let rd_z = _mm_set_ps1(ray.direction.z);
        // current indices being processed (little endian ordering)
        let mut index = _mm_set_epi32(3, 2, 1, 0);
        // loop over 4 spheres at a time
        for (((centre_x, centre_y), centre_z), radius_sq) in self
            .centre_x
            .iter()
            .zip(self.centre_y.iter())
            .zip(self.centre_z.iter())
            .zip(self.radius_sq.iter())
        {
            // load sphere centres
            let c_x = _mm_load_ps(centre_x.0.as_ptr());
            let c_y = _mm_load_ps(centre_y.0.as_ptr());
            let c_z = _mm_load_ps(centre_z.0.as_ptr());
            // load radius_sq
            let r_sq = _mm_load_ps(radius_sq.0.as_ptr());
            // let co = centre - ray.origin
            let co_x = _mm_sub_ps(c_x, ro_x);
            let co_y = _mm_sub_ps(c_y, ro_y);
            let co_z = _mm_sub_ps(c_z, ro_z);
            // TODO: write a dot product helper
            // let nb = dot(co, ray.direction);
            let nb = _mm_mul_ps(co_x, rd_x);
            let nb = _mm_add_ps(nb, _mm_mul_ps(co_y, rd_y));
            let nb = _mm_add_ps(nb, _mm_mul_ps(co_z, rd_z));
            // let c = dot(co, co) - radius_sq;
            let c = _mm_mul_ps(co_x, co_x);
            let c = _mm_add_ps(c, _mm_mul_ps(co_y, co_y));
            let c = _mm_add_ps(c, _mm_mul_ps(co_z, co_z));
            let c = _mm_sub_ps(c, r_sq);
            // let discriminant = nb * nb - c;
            let discr = _mm_sub_ps(_mm_mul_ps(nb, nb), c);
            // if discr > 0.0
            let pos_discr = _mm_cmpgt_ps(discr, _mm_set_ps1(0.0));
            if _mm_movemask_ps(pos_discr) != 0 {
                // let discr_sqrt = discr.sqrt();
                let discr_sqrt = _mm_sqrt_ps(discr);
                // let t0 = nb - discr_sqrt;
                // let t1 = nb + discr_sqrt;
                let t0 = _mm_sub_ps(nb, discr_sqrt);
                let t1 = _mm_add_ps(nb, discr_sqrt);
                // let t = if t0 > t_min { t0 } else { t1 };
                let t = _mm_blendv_ps(t1, t0, _mm_cmpgt_ps(t0, t_min));
                // from rygs opts
                // bool4 msk = discrPos & (t > tMin4) & (t < hitT);
                let mask = _mm_and_ps(
                    pos_discr,
                    _mm_and_ps(_mm_cmpgt_ps(t, t_min), _mm_cmplt_ps(t, hit_t)),
                );
                // hit_index = mask ? index : hit_index;
                hit_index = _mm_blendv_epi8(hit_index, index, _mm_castps_si128(mask));
                // hit_t = mask ? t : hit_t;
                hit_t = _mm_blendv_ps(hit_t, t, mask);
            }
            // increment indices
            index = _mm_add_epi32(index, _mm_set1_epi32(4));
        }

        // copy results out into scalar code to get return value (if any)
        let mut hit_t_array = [t_max; F32XN_LANES];
        _mm_store_ps(hit_t_array.as_mut_ptr() as *mut f32, hit_t);
        let (hit_t_lane, hit_t_scalar) = hit_t_array.iter().enumerate().fold(
            (self.len, t_max),
            |result, t| if *t.1 < result.1 { (t.0, *t.1) } else { result },
        );
        // .min_by(|x, y| {
        //     // PartialOrd strikes again
        //     use std::cmp::Ordering;
        //     if x.1 < y.1 {
        //         Ordering::Less
        //     } else if x.1 > y.1 {
        //         Ordering::Greater
        //     } else {
        //         Ordering::Equal
        //     }
        // })
        // .unwrap();
        // let hit_t_scalar = *hit_t_scalar;
        if hit_t_scalar < t_max {
            let mut hit_index_array = [-1i32; F32XN_LANES];
            _mm_storeu_si128(hit_index_array.as_mut_ptr() as *mut __m128i, hit_index);
            let hit_index_scalar = hit_index_array[hit_t_lane] as usize;
            // TODO: does compiler turn this into a bit shift?
            let array_index = hit_index_scalar / F32XN_LANES;
            let lane_index = hit_index_scalar - array_index;
            debug_assert!(hit_index_scalar < self.len);
            let point = ray.point_at_parameter(hit_t_scalar);
            let normal = vec3(
                point.x - self.centre_x.get_unchecked(array_index).0.get_unchecked(lane_index),
                point.y - self.centre_y.get_unchecked(array_index).0.get_unchecked(lane_index),
                point.z - self.centre_z.get_unchecked(array_index).0.get_unchecked(lane_index),
            ) * *self.radius_inv.get_unchecked(array_index).0.get_unchecked(lane_index);
            let material = &self.material.get_unchecked(array_index).get_unchecked(lane_index);
            Some((RayHit { point, normal }, material))
        } else {
            None
        }
    }
}