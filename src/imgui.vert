#version 450
#extension GL_ARB_separate_shader_objects : enable

layout(location = 0) in vec2 position;
layout(location = 1) in vec2 uv;
layout(location = 2) in uint color;
layout(location = 0) out vec4 fragColor;
layout(location = 1) out vec2 fragUv;

layout(set = 0, binding = 0) uniform UniformBufferObject {
    mat4 projection;
} ubo;

out gl_PerVertex {
    vec4 gl_Position;
};
void main() {
    gl_Position = ubo.projection * vec4(position, 0.0, 1.0);
    fragColor = unpackUnorm4x8(color);
    fragUv = uv;
}