#import <Foundation/Foundation.h>
#import <ModelIO/ModelIO.h>
#import <CoreGraphics/CoreGraphics.h>
#import <objc/message.h>
#include <algorithm>
#include <cmath>
#include <unordered_set>
#include <cstring>
#include <vector>

struct Raster {
    const uint8_t *pixels;
    uint32_t width, height, wrapS, wrapT, magFilter, minFilter, texCoord;
};
struct Material {
    float rgba[4], emissive[3], metallic, roughness, occlusionStrength, alphaCutoff;
    bool doubleSided;
    Raster base, opacity, metal, rough, normal, occlusion, emission;
};
struct Part {
    const uint32_t *indices;
    size_t count, materialId;
};
using EmitMaterial = bool (*)(void *, size_t, const char *, const Material *);
using EmitMesh = bool (*)(void *, size_t, const char *, const float *, const float *, const float *,
                          const float *, size_t, const Part *, size_t);
using EmitNode = bool (*)(void *, size_t, const char *, const float *);

struct Output {
    void *context;
    EmitMaterial material;
    EmitMesh mesh;
    EmitNode node;
    std::unordered_set<size_t> meshes, emptyMeshes, materials;
};

static size_t identity(id object) {
    return reinterpret_cast<size_t>((__bridge const void *)object);
}

static uint32_t wrap(MDLMaterialTextureWrapMode mode) {
    switch (mode) {
        case MDLMaterialTextureWrapModeClamp: return 33071;
        case MDLMaterialTextureWrapModeMirror: return 33648;
        default: return 10497;
    }
}

static Raster rasterize(MDLTextureSampler *sampler, bool color, bool hasUV1,
                        const bool repeatU[2], const bool repeatV[2], std::vector<uint8_t> &bytes) {
    MDLTexture *texture = sampler.texture;
    if (!texture || texture.isCube) return {};
    CGImageRef image = [texture imageFromTexture];
    if (!image) return {};
    CGImageRetain(image); // Model I/O owns the returned image.
    const size_t w = CGImageGetWidth(image), h = CGImageGetHeight(image);
    const size_t edge = std::max(w, h);
    if (!w || !h) {
        CGImageRelease(image);
        return {};
    }
    const double scale = std::min(1.0, 2048.0 / edge);
    const uint32_t width = std::max(1L, std::lround(w * scale));
    const uint32_t height = std::max(1L, std::lround(h * scale));
    CGColorSpaceRef rgb = CGColorSpaceCreateWithName(color ? kCGColorSpaceSRGB : kCGColorSpaceLinearSRGB);
    // Scalar and vector data use the stored numeric samples. Reinterpret tagged
    // RGB/gray images as linear before CoreGraphics performs any conversion.
    if (!color) {
        CGColorSpaceRef source = CGImageGetColorSpace(image);
        CGColorSpaceRef linear = nullptr;
        if (source && CGColorSpaceGetModel(source) == kCGColorSpaceModelRGB) {
            linear = CGColorSpaceRetain(rgb);
        } else if (source && CGColorSpaceGetModel(source) == kCGColorSpaceModelMonochrome) {
            linear = CGColorSpaceCreateWithName(kCGColorSpaceLinearGray);
        }
        if (linear) {
            CGImageRef copy = CGImageCreateCopyWithColorSpace(image, linear);
            CGColorSpaceRelease(linear);
            if (copy) {
                CGImageRelease(image);
                image = copy;
            }
        }
    }
    bytes.resize(size_t(width) * height * 4);
    CGContextRef bitmap = CGBitmapContextCreate(bytes.data(), width, height, 8, width * 4, rgb,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(rgb);
    if (!bitmap) {
        CGImageRelease(image);
        return {};
    }
    CGContextSetBlendMode(bitmap, kCGBlendModeCopy);
    CGContextSetInterpolationQuality(bitmap, kCGInterpolationHigh);
    CGContextDrawImage(bitmap, CGRectMake(0, 0, width, height), image);
    CGContextRelease(bitmap);
    CGImageRelease(image);
    // glTF stores straight (unassociated) alpha.
    for (size_t i = 0; i < bytes.size(); i += 4) {
        const unsigned alpha = bytes[i + 3];
        if (alpha && alpha < 255) {
            for (size_t c = 0; c < 3; c++) {
                bytes[i + c] = std::min(255u, (bytes[i + c] * 255u + alpha / 2) / alpha);
            }
        }
    }
    SEL mappingSel = NSSelectorFromString(@"mappingChannel");
    const NSUInteger channel = (mappingSel && [sampler respondsToSelector:mappingSel])
        ? reinterpret_cast<NSUInteger (*)(id, SEL)>(objc_msgSend)(sampler, mappingSel)
        : 0u;
    const uint32_t texCoord = (channel == 1u && hasUV1) ? 1u : 0u;
    MDLTextureFilter *filter = sampler.hardwareFilter;
    const bool nearest = filter && filter.minFilter == MDLMaterialTextureFilterModeNearest;
    uint32_t min = nearest ? 9728 : 9729;
    if (!filter || filter.mipFilter == MDLMaterialMipMapFilterModeLinear) min = nearest ? 9986 : 9987;
    else if (filter.mipFilter == MDLMaterialMipMapFilterModeNearest) min = nearest ? 9984 : 9985;
    uint32_t wrapS = filter ? wrap(filter.sWrapMode) : 10497u;
    uint32_t wrapT = filter ? wrap(filter.tWrapMode) : 10497u;
    // Some USDZ assets author UsdPreviewSurface with clamp while their RealityKit
    // MaterialX twin uses repeat across UVs that tile multiple times; honor repeat
    // when the bound submesh spans more than 1.5 tiles along that axis.
    if (wrapS == 33071u && repeatU[texCoord]) wrapS = 10497u;
    if (wrapT == 33071u && repeatV[texCoord]) wrapT = 10497u;
    return {bytes.data(), width, height, wrapS, wrapT,
            filter && filter.magFilter == MDLMaterialTextureFilterModeNearest ? 9728u : 9729u,
            min, texCoord};
}

static MDLMaterialProperty *scalarProperty(MDLMaterial *material, MDLMaterialSemantic semantic) {
    // `propertiesWithSemantic:` returns authored material properties before the
    // scattering function's built-in defaults (e.g. `diffuseColor` before `baseColor`).
    for (MDLMaterialProperty *property in [material propertiesWithSemantic:semantic]) {
        switch (property.type) {
            case MDLMaterialPropertyTypeFloat:
            case MDLMaterialPropertyTypeFloat3:
            case MDLMaterialPropertyTypeFloat4:
            case MDLMaterialPropertyTypeColor:
                return property;
            default: break;
        }
    }
    return nil;
}

static float scalar(MDLMaterial *material, MDLMaterialSemantic semantic, float fallback) {
    MDLMaterialProperty *property = scalarProperty(material, semantic);
    return property ? property.floatValue : fallback;
}

static void colorFactor(MDLMaterialProperty *property, float *rgba, size_t count) {
    if (property.type == MDLMaterialPropertyTypeFloat3) {
        vector_float3 value = property.float3Value;
        for (size_t i = 0; i < 3; i++) rgba[i] = value[i];
    } else if (property.type == MDLMaterialPropertyTypeFloat4) {
        vector_float4 value = property.float4Value;
        for (size_t i = 0; i < count; i++) rgba[i] = value[i];
    } else if (property.type == MDLMaterialPropertyTypeColor && property.color) {
        CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
        if (!space) return;
        CGColorRef linear = CGColorCreateCopyByMatchingToColorSpace(space, kCGRenderingIntentDefault, property.color, nullptr);
        if (linear) {
            const CGFloat *channels = CGColorGetComponents(linear);
            for (size_t i = 0; i < count; i++) rgba[i] = channels[i];
            CGColorRelease(linear);
        }
        CGColorSpaceRelease(space);
    } else if (property.type == MDLMaterialPropertyTypeFloat) {
        for (size_t i = 0; i < 3; i++) rgba[i] = property.floatValue;
    }
}

static Raster map(MDLMaterial *material, MDLMaterialSemantic semantic, bool color, bool hasUV1,
                  const bool repeatU[2], const bool repeatV[2], std::vector<uint8_t> &bytes) {
    for (MDLMaterialProperty *property in [material propertiesWithSemantic:semantic]) {
        if (property.type != MDLMaterialPropertyTypeTexture) continue;
        Raster raster = rasterize(property.textureSamplerValue, color, hasUV1, repeatU, repeatV, bytes);
        if (raster.pixels) return raster;
    }
    return {};
}

static size_t materialKey(MDLMaterial *material, bool hasUV1, const bool repeatU[2], const bool repeatV[2]) {
    if (!material) return 0;
    const size_t flags = (hasUV1 ? 1u : 0u)
        | (repeatU[0] ? 2u : 0u) | (repeatV[0] ? 4u : 0u)
        | (repeatU[1] ? 8u : 0u) | (repeatV[1] ? 16u : 0u);
    return (identity(material) << 5) ^ (identity(material) >> 59) ^ flags;
}

static bool emitMaterial(MDLMaterial *material, bool hasUV1, const bool repeatU[2], const bool repeatV[2],
                         Output &out, size_t &materialId) {
    materialId = materialKey(material, hasUV1, repeatU, repeatV);
    if (!material || !out.materials.insert(materialId).second) return true;
    @autoreleasepool {
        std::vector<uint8_t> pixels[7];
        Material m{};
        m.base = map(material, MDLMaterialSemanticBaseColor, true, hasUV1, repeatU, repeatV, pixels[0]);
        m.opacity = map(material, MDLMaterialSemanticOpacity, false, hasUV1, repeatU, repeatV, pixels[1]);
        m.metal = map(material, MDLMaterialSemanticMetallic, false, hasUV1, repeatU, repeatV, pixels[2]);
        m.rough = map(material, MDLMaterialSemanticRoughness, false, hasUV1, repeatU, repeatV, pixels[3]);
        m.normal = map(material, MDLMaterialSemanticTangentSpaceNormal, false, hasUV1, repeatU, repeatV, pixels[4]);
        m.occlusion = map(material, MDLMaterialSemanticAmbientOcclusion, false, hasUV1, repeatU, repeatV, pixels[5]);
        m.emission = map(material, MDLMaterialSemanticEmission, true, hasUV1, repeatU, repeatV, pixels[6]);
        std::fill_n(m.rgba, 4, 1.f);
        MDLMaterialProperty *base = scalarProperty(material, MDLMaterialSemanticBaseColor);
        if (base && (!m.base.pixels || ![base.name isEqualToString:@"baseColor"])) colorFactor(base, m.rgba, 4);
        MDLMaterialProperty *emission = scalarProperty(material, MDLMaterialSemanticEmission);
        if (m.emission.pixels) std::fill_n(m.emissive, 3, 1.f);
        if (emission && (!m.emission.pixels || ![emission.name isEqualToString:@"emission"])) colorFactor(emission, m.emissive, 3);
        m.rgba[3] *= scalar(material, MDLMaterialSemanticOpacity, 1.f);
        // Model I/O supplies scalar defaults even when the USD input is a map.
        // The map replaces the default scalar for that input.
        m.metallic = m.metal.pixels ? 1.f : scalar(material, MDLMaterialSemanticMetallic, 0.f);
        m.roughness = m.rough.pixels ? 1.f : scalar(material, MDLMaterialSemanticRoughness, 1.f);
        m.occlusionStrength = scalar(material, MDLMaterialSemanticAmbientOcclusionScale, 1.f);
        m.alphaCutoff = [material propertyNamed:@"opacityThreshold"].floatValue;
        m.doubleSided = material.materialFace == MDLMaterialFaceDoubleSided
            || [material propertyNamed:@"doubleSided"].floatValue != 0.f;
        return out.material(out.context, materialId, material.name.UTF8String, &m);
    }
}

static bool attribute(MDLMesh *mesh, NSString *name, MDLVertexFormat format, size_t width,
                      std::vector<float> &values) {
    __attribute__((objc_precise_lifetime)) MDLVertexAttributeData *data = [mesh vertexAttributeDataForAttributeNamed:name asFormat:format];
    if (!data || !data.dataStart || data.stride < width * sizeof(float)) return false;
    const size_t count = mesh.vertexCount;
    if (count == 0 || (count - 1) > (SIZE_MAX - width * sizeof(float)) / data.stride ||
        (count - 1) * data.stride + width * sizeof(float) > data.bufferSize) return false;
    values.resize(count * width);
    const auto *start = static_cast<const uint8_t *>(data.dataStart);
    for (size_t i = 0; i < count; i++) {
        std::memcpy(values.data() + i * width, start + i * data.stride, width * sizeof(float));
    }
    return true;
}

static bool drawMesh(MDLMesh *mesh, NSString *name, matrix_float4x4 matrix, Output &out) {
    const size_t id = identity(mesh);
    if (out.emptyMeshes.count(id)) return true;
    float transform[16];
    std::memcpy(transform, &matrix, sizeof(transform));
    if (!out.meshes.insert(id).second) return out.node(out.context, id, name.UTF8String, transform);
    const size_t count = mesh.vertexCount;
    if (count == 0) {
        out.emptyMeshes.insert(id);
        return true;
    }
    if (count > 30000000) return false;
    std::vector<float> positions, normals, uv, uv1;
    if (!attribute(mesh, MDLVertexAttributePosition, MDLVertexFormatFloat3, 3, positions)) return false;
    bool hasNormals = attribute(mesh, MDLVertexAttributeNormal, MDLVertexFormatFloat3, 3, normals);
    bool hasUV = attribute(mesh, MDLVertexAttributeTextureCoordinate, MDLVertexFormatFloat2, 2, uv);
    bool hasUV1 = attribute(mesh, @"textureCoordinate1", MDLVertexFormatFloat2, 2, uv1);
    // MDL coordinates have bottom-left origin. CGImage and glTF use
    // top-left image rows, so change V here exactly once.
    if (hasUV) {
        for (size_t i = 1; i < uv.size(); i += 2) uv[i] = 1.f - uv[i];
    }
    if (hasUV1) {
        for (size_t i = 1; i < uv1.size(); i += 2) uv1[i] = 1.f - uv1[i];
    }
    auto uvSpan = [count](const std::vector<float> &coords, const std::vector<uint32_t> &indices,
                          bool &tileU, bool &tileV) {
        if (coords.empty() || indices.empty()) return;
        float minU = INFINITY, maxU = -INFINITY, minV = INFINITY, maxV = -INFINITY;
        for (uint32_t idx : indices) {
            if (idx >= count) continue;
            const float u = coords[size_t(idx) * 2];
            const float v = coords[size_t(idx) * 2 + 1];
            if (std::isfinite(u)) { minU = std::min(minU, u); maxU = std::max(maxU, u); }
            if (std::isfinite(v)) { minV = std::min(minV, v); maxV = std::max(maxV, v); }
        }
        if (maxU - minU > 1.5f) tileU = true;
        if (maxV - minV > 1.5f) tileV = true;
    };
    std::vector<std::vector<uint32_t>> indexBuffers;
    std::vector<Part> parts;
    size_t indexCount = 0;
    for (MDLSubmesh *part in mesh.submeshes) {
        const size_t n = part.indexCount;
        if (n == 0 || part.geometryType <= MDLGeometryTypeLines) continue;
        if (n > 30000000) return false;
        const size_t width = part.indexType / 8;
        if ((width != 1 && width != 2 && width != 4) || n * width > part.indexBuffer.length) return false;
        __attribute__((objc_precise_lifetime)) MDLMeshBufferMap *map = [part.indexBuffer map];
        const auto *data = static_cast<const uint8_t *>(map.bytes);
        if (!data) return false;
        std::vector<uint32_t> indices(n);
        for (size_t i = 0; i < n; i++) std::memcpy(&indices[i], data + i * width, width);
        std::vector<uint32_t> triangles;
        const std::vector<uint32_t> *output = &indices;
        switch (part.geometryType) {
            case MDLGeometryTypeTriangles:
                if (n % 3 != 0) return false;
                break;
            case MDLGeometryTypeQuads:
                if (n % 4 != 0 || n / 4 > 5000000) return false;
                for (size_t i = 0; i < n; i += 4) {
                    triangles.insert(triangles.end(), {indices[i], indices[i+1], indices[i+2],
                                                      indices[i], indices[i+2], indices[i+3]});
                }
                output = &triangles;
                break;
            case MDLGeometryTypeTriangleStrips:
                if (n > 10000002) return false;
                for (size_t i = 2; i < n; i++) {
                    triangles.insert(triangles.end(), {indices[i-2 + (i % 2)], indices[i-1 - (i % 2)], indices[i]});
                }
                output = &triangles;
                break;
            default:
                return false;
        }
        if (output->empty()) continue;
        indexCount += output->size();
        if (indexCount > 30000000) return false;
        MDLMaterial *material = part.material;
        bool repeatU[2] = {false, false};
        bool repeatV[2] = {false, false};
        uvSpan(uv, *output, repeatU[0], repeatV[0]);
        uvSpan(uv1, *output, repeatU[1], repeatV[1]);
        size_t matId = 0;
        if (!emitMaterial(material, hasUV1, repeatU, repeatV, out, matId)) return false;
        indexBuffers.push_back(output == &indices ? std::move(indices) : std::move(triangles));
        const auto &buffer = indexBuffers.back();
        parts.push_back({buffer.data(), buffer.size(), matId});
    }
    if (parts.empty()) {
        out.emptyMeshes.insert(id);
        return true;
    }
    return out.mesh(out.context, id, mesh.name.UTF8String, positions.data(),
                    hasNormals ? normals.data() : nullptr, hasUV ? uv.data() : nullptr,
                    hasUV1 ? uv1.data() : nullptr, count, parts.data(), parts.size())
        && out.node(out.context, id, name.UTF8String, transform);
}

static bool walk(MDLObject *object, matrix_float4x4 parent, matrix_float4x4 axis, double time, size_t depth,
                 size_t &visited, Output &out, NSString *instanceName = nil) {
    if (depth == 64 || ++visited > 65536) return false;
    id<MDLTransformComponent> transform = object.transform;
    matrix_float4x4 world = parent;
    if (transform) {
        matrix_float4x4 local = [transform respondsToSelector:@selector(localTransformAtTime:)]
            ? [transform localTransformAtTime:time] : transform.matrix;
        world = transform.resetsTransform ? local : simd_mul(parent, local);
    }
    if ([object isKindOfClass:[MDLMesh class]] && !drawMesh((MDLMesh *)object, instanceName ?: object.name, simd_mul(axis, world), out)) return false;
    if (object.instance && !walk(object.instance, world, axis, time, depth + 1, visited, out, instanceName ?: object.name)) return false;
    for (MDLObject *child in object.children.objects) {
        if (!walk(child, world, axis, time, depth + 1, visited, out)) return false;
    }
    return true;
}

extern "C" bool pm_scene3d_modelio_supports(const char *extension) {
    @autoreleasepool {
        NSString *text = [NSString stringWithUTF8String:extension];
        return text && [MDLAsset canImportFileExtension:text];
    }
}

extern "C" bool pm_scene3d_modelio_read_scene(const char *path, void *context,
                                         EmitMaterial material, EmitMesh mesh, EmitNode node) {
    @autoreleasepool {
        @try {
            try {
                NSString *filename = [NSString stringWithUTF8String:path];
                if (!filename) return false;
                NSURL *url = [NSURL fileURLWithPath:filename];
                NSError *error = nil;
                MDLAsset *asset = [[MDLAsset alloc] initWithURL:url vertexDescriptor:nil
                    bufferAllocator:nil preserveTopology:NO error:&error];
                if (!asset || error || asset.count == 0 || asset.count > 65536) return false;
                [asset loadTextures];
                Output out{context, material, mesh, node};
                matrix_float4x4 axis = matrix_identity_float4x4;
                vector_float3 up = asset.upAxis;
                if (std::abs(up.z) > 0.5f) {
                    float sign = up.z > 0.f ? 1.f : -1.f;
                    axis.columns[1] = (vector_float4){0, 0, -sign, 0};
                    axis.columns[2] = (vector_float4){0, sign, 0, 0};
                } else if (std::abs(up.x) > 0.5f) {
                    float sign = up.x > 0.f ? 1.f : -1.f;
                    axis.columns[0] = (vector_float4){0, sign, 0, 0};
                    axis.columns[1] = (vector_float4){-sign, 0, 0, 0};
                }
                size_t visited = 0;
                for (NSUInteger i = 0; i < asset.count; i++) {
                    if (!walk([asset objectAtIndex:i], matrix_identity_float4x4, axis, asset.startTime,
                              0, visited, out)) return false;
                }
                return true;
            } catch (...) {
                return false;
            }
        } @catch (NSException *exception) {
            return false;
        }
    }
}
