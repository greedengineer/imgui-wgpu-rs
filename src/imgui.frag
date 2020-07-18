#version 450
#extension GL_ARB_separate_shader_objects : enable

layout(location = 0) in vec4 fragColor;
layout(location = 1) in vec2 fragUv;
layout(location = 0) out vec4 outColor;

layout(set = 1, binding = 0) uniform texture2D tex;
layout(set = 1, binding = 1) uniform sampler texSampler;

void main() {
    vec4 texColor = texture(sampler2D(tex,texSampler), fragUv);
    outColor = fragColor * texColor;
}