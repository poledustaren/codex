const BEDROCK_MANTLE_SUPPORTED_REGIONS: [&str; 12] = [
    "us-east-2",
    "us-east-1",
    "us-west-2",
    "ap-southeast-3",
    "ap-south-1",
    "ap-northeast-1",
    "eu-central-1",
    "eu-west-1",
    "eu-west-2",
    "eu-south-1",
    "eu-north-1",
    "sa-east-1",
];
const BEDROCK_GOV_CLOUD_SUPPORTED_REGIONS: [&str; 2] = ["us-gov-east-1", "us-gov-west-1"];

/// Returns whether Amazon Bedrock Mantle is available in `region`.
pub fn is_supported_amazon_bedrock_region(region: &str) -> bool {
    BEDROCK_MANTLE_SUPPORTED_REGIONS.contains(&region) || is_amazon_bedrock_gov_cloud_region(region)
}

/// Returns whether `region` is a supported Amazon Bedrock GovCloud region.
pub fn is_amazon_bedrock_gov_cloud_region(region: &str) -> bool {
    BEDROCK_GOV_CLOUD_SUPPORTED_REGIONS.contains(&region)
}
