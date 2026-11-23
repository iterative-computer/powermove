#import <Foundation/Foundation.h>
#import <ModelIO/ModelIO.h>
#import <CoreGraphics/CoreGraphics.h>
#include <cmath>
#include <cstring>
#include <vector>

using Emit = bool (*)(void *, const char *, const float *, const float *, const float *,
                      size_t, const uint32_t *, size_t, const float *, const char *, const float *);

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

static bool drawMesh(MDLMesh *mesh, matrix_float4x4 matrix, void *context, Emit emit) {
    const size_t count = mesh.vertexCount;
    if (count == 0) return true;
    if (count > 30000000) return false;
    std::vector<float> positions, normals, uv;
    if (!attribute(mesh, MDLVertexAttributePosition, MDLVertexFormatFloat3, 3, positions)) return false;
    bool hasNormals = attribute(mesh, MDLVertexAttributeNormal, MDLVertexFormatFloat3, 3, normals);
    bool hasUV = attribute(mesh, MDLVertexAttributeTextureCoordinate, MDLVertexFormatFloat2, 2, uv);
    if (hasUV) {
        for (size_t i = 1; i < uv.size(); i += 2) uv[i] = 1.f - uv[i];
    }
    float transform[16];
    std::memcpy(transform, &matrix, sizeof(transform));
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
        float rgba[4] = {1, 1, 1, 1};
        MDLMaterialProperty *color = [part.material propertyWithSemantic:MDLMaterialSemanticBaseColor];
        if (color.type == MDLMaterialPropertyTypeFloat3) {
            vector_float3 rgb = color.float3Value;
            for (size_t i = 0; i < 3; i++) rgba[i] = rgb[i];
        } else if (color.type == MDLMaterialPropertyTypeFloat4) {
            vector_float4 value = color.float4Value;
            for (size_t i = 0; i < 4; i++) rgba[i] = value[i];
        } else if (color.type == MDLMaterialPropertyTypeColor && color.color) {
            CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
            if (space) {
                CGColorRef linear = CGColorCreateCopyByMatchingToColorSpace(space, kCGRenderingIntentDefault, color.color, nullptr);
                if (linear) {
                    const CGFloat *channels = CGColorGetComponents(linear);
                    for (size_t i = 0; i < 4; i++) rgba[i] = channels[i];
                    CGColorRelease(linear);
                }
                CGColorSpaceRelease(space);
            }
        }
        if (!emit(context, mesh.name.UTF8String, positions.data(), hasNormals ? normals.data() : nullptr,
                  hasUV ? uv.data() : nullptr, count, output->data(), output->size(), transform,
                  part.material.name.UTF8String, rgba)) return false;
    }
    return true;
}

static bool walk(MDLObject *object, matrix_float4x4 parent, matrix_float4x4 axis, double time, size_t depth,
                 size_t &visited, void *context, Emit emit) {
    if (depth == 64 || ++visited > 65536) return false;
    id<MDLTransformComponent> transform = object.transform;
    matrix_float4x4 world = parent;
    if (transform) {
        matrix_float4x4 local = [transform respondsToSelector:@selector(localTransformAtTime:)]
            ? [transform localTransformAtTime:time] : transform.matrix;
        world = transform.resetsTransform ? local : simd_mul(parent, local);
    }
    if ([object isKindOfClass:[MDLMesh class]] && !drawMesh((MDLMesh *)object, simd_mul(axis, world), context, emit)) return false;
    if (object.instance && !walk(object.instance, world, axis, time, depth + 1, visited, context, emit)) return false;
    for (MDLObject *child in object.children.objects) {
        if (!walk(child, world, axis, time, depth + 1, visited, context, emit)) return false;
    }
    return true;
}

extern "C" bool pm_scene3d_modelio_supports(const char *extension) {
    @autoreleasepool {
        NSString *text = [NSString stringWithUTF8String:extension];
        return text && [MDLAsset canImportFileExtension:text];
    }
}

extern "C" bool pm_scene3d_modelio_read(const char *path, void *context, Emit emit) {
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
                              0, visited, context, emit)) return false;
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
